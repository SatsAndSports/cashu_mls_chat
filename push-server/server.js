#!/usr/bin/env node

const express = require('express');
const webpush = require('web-push');
const cors = require('cors');
const WebSocket = require('ws');

// Configuration
const PORT = process.env.PORT || 3000;
const VAPID_PUBLIC_KEY = process.env.VAPID_PUBLIC_KEY;
const VAPID_PRIVATE_KEY = process.env.VAPID_PRIVATE_KEY;
const VAPID_SUBJECT = process.env.VAPID_SUBJECT || 'mailto:admin@example.com';

// Logging configuration
// Levels: 'silent', 'error', 'info', 'debug'
const LOG_LEVEL = process.env.LOG_LEVEL || 'info';
const LOG_LEVELS = { silent: 0, error: 1, info: 2, debug: 3 };
const currentLogLevel = LOG_LEVELS[LOG_LEVEL] || LOG_LEVELS.info;

const logger = {
  error: (...args) => {
    if (currentLogLevel >= LOG_LEVELS.error) console.error(...args);
  },
  info: (...args) => {
    if (currentLogLevel >= LOG_LEVELS.info) console.log(...args);
  },
  debug: (...args) => {
    if (currentLogLevel >= LOG_LEVELS.debug) console.log(...args);
  }
};

// Validate VAPID keys
if (!VAPID_PUBLIC_KEY || !VAPID_PRIVATE_KEY) {
  logger.error('ERROR: VAPID keys not configured!');
  logger.error('Run: npm run generate-vapid');
  logger.error('Then add keys to .env file');
  process.exit(1);
}

// Configure web-push
webpush.setVapidDetails(
  VAPID_SUBJECT,
  VAPID_PUBLIC_KEY,
  VAPID_PRIVATE_KEY
);

// Express app
const app = express();
app.use(cors());
app.use(express.json());

// In-memory storage (use Redis/DB for production)
// Structure: { userPubkey: { subscription: {...}, groupIds: [...], subscriptionTimestamp: ... } }
const subscriptions = new Map();

// Nostr relay connections
const relayConnections = new Map();

// Track sent notifications to avoid duplicates
// Structure: Map<"eventId:pubkey", { relay: string, timestamp: number }>
const sentNotifications = new Map();

// Clean up old notification tracking entries every 5 minutes
setInterval(() => {
  const fiveMinutesAgo = Date.now() - (5 * 60 * 1000);
  for (const [key, value] of sentNotifications.entries()) {
    if (value.timestamp < fiveMinutesAgo) {
      sentNotifications.delete(key);
    }
  }
  if (sentNotifications.size > 0) {
    logger.debug(`🧹 Cleaned up old notification tracking entries. Current size: ${sentNotifications.size}`);
  }
}, 5 * 60 * 1000);

// Connect to Nostr relay
function connectToRelay(relayUrl) {
  if (relayConnections.has(relayUrl)) {
    return relayConnections.get(relayUrl);
  }

  logger.debug(`📡 Connecting to relay: ${relayUrl}`);

  const ws = new WebSocket(relayUrl);
  const relay = {
    ws,
    url: relayUrl,
    connected: false,
    subscriptions: new Set()
  };

  ws.on('open', () => {
    logger.debug(`✅ Connected to ${relayUrl}`);
    relay.connected = true;
    updateRelaySubscriptions(relay);
  });

  ws.on('message', (data) => {
    handleRelayMessage(relay, data);
  });

  ws.on('close', () => {
    logger.debug(`❌ Disconnected from ${relayUrl}`);
    relay.connected = false;
    relayConnections.delete(relayUrl);

    // Reconnect after 5 seconds
    setTimeout(() => {
      logger.debug(`🔄 Reconnecting to ${relayUrl}...`);
      connectToRelay(relayUrl);
    }, 5000);
  });

  ws.on('error', (err) => {
    logger.error(`Error with ${relayUrl}:`, err.message);
  });

  relayConnections.set(relayUrl, relay);
  return relay;
}

// Update relay subscriptions based on active user subscriptions
function updateRelaySubscriptions(relay) {
  if (!relay.connected) return;

  // Collect all group IDs and user pubkeys we need to monitor
  const allGroupIds = new Set();
  const allUserPubkeys = new Set();

  for (const [pubkey, data] of subscriptions.entries()) {
    data.groupIds.forEach(id => allGroupIds.add(id));
    allUserPubkeys.add(pubkey);
  }

  if (allGroupIds.size === 0 && allUserPubkeys.size === 0) {
    logger.debug(`No active subscriptions for ${relay.url}`);
    return;
  }

  // Subscribe to both Kind 445 (messages) and Kind 444 (welcomes)
  // Only get messages from now onwards (not historical)
  const subId = 'mdk-push';
  const since = Math.floor(Date.now() / 1000);
  const filters = [];

  // Kind 445: Group messages
  if (allGroupIds.size > 0) {
    filters.push({
      kinds: [445],
      '#h': Array.from(allGroupIds),
      since: since
    });
  }

  // Kind 444: Welcome messages (invites)
  if (allUserPubkeys.size > 0) {
    filters.push({
      kinds: [444],
      '#p': Array.from(allUserPubkeys),
      since: since
    });
  }

  const subscribeMsg = JSON.stringify(['REQ', subId, ...filters]);
  relay.ws.send(subscribeMsg);

  logger.debug(`📬 Subscribed to ${allGroupIds.size} groups and ${allUserPubkeys.size} users on ${relay.url}`);
  logger.debug(`   Filter: ${JSON.stringify(filters)}`);
}

// Handle message from Nostr relay
function handleRelayMessage(relay, data) {
  try {
    const msg = JSON.parse(data.toString());

    // Debug: log all message types
    logger.debug(`[${relay.url}] Received: ${msg[0]}`);

    if (msg[0] === 'EVENT') {
      const event = msg[2];
      logger.debug(`[${relay.url}] EVENT kind=${event.kind} id=${event.id.substring(0, 8)}...`);
      handleNostrEvent(event, relay);
    } else if (msg[0] === 'EOSE') {
      logger.debug(`[${relay.url}] End of stored events`);
    } else if (msg[0] === 'NOTICE') {
      logger.debug(`[${relay.url}] NOTICE: ${msg[1]}`);
    }
  } catch (err) {
    logger.error('Error parsing relay message:', err.message);
  }
}

// Handle Nostr event and send push notifications
async function handleNostrEvent(event, relay) {
  if (event.kind === 445) {
    // Kind 445: Group message
    await handleGroupMessage(event, relay);
  } else if (event.kind === 444) {
    // Kind 444: Welcome message (invite)
    await handleWelcomeMessage(event, relay);
  }
}

// Handle Kind 445 (group message)
async function handleGroupMessage(event, relay) {
  // Extract group ID from h-tag
  const hTag = event.tags.find(tag => tag[0] === 'h');
  if (!hTag || !hTag[1]) return;

  const groupId = hTag[1];

  logger.debug(`📨 New message in group ${groupId.substring(0, 16)}... from ${relay.url}`);

  // Format timestamp
  const timestamp = new Date(event.created_at * 1000);
  const timeStr = timestamp.toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit' });

  // Extract relay domain
  const relayDomain = relay.url.replace('wss://', '').replace('ws://', '').split('/')[0];

  // Find all users subscribed to this group
  for (const [pubkey, data] of subscriptions.entries()) {
    // Don't notify the sender
    if (pubkey === event.pubkey) continue;

    // Check if user is subscribed to this group
    if (!data.groupIds.includes(groupId)) continue;

    // Check if we already sent a notification for this event to this user
    const notificationKey = `${event.id}:${pubkey}`;
    if (sentNotifications.has(notificationKey)) {
      const firstRelay = sentNotifications.get(notificationKey).relay;
      logger.debug(`  ⏭️  Skipping duplicate (already sent via ${firstRelay})`);
      continue;
    }

    // Record that we're sending this notification
    sentNotifications.set(notificationKey, {
      relay: relay.url,
      timestamp: Date.now()
    });

    // Send push notification
    await sendPushNotification(pubkey, data.subscription, {
      title: 'New message',
      body: `Group: ${groupId.substring(0, 12)}...\nTime: ${timeStr}\nRelay: ${relayDomain}`,
      tag: `message-${groupId}`,
      data: {
        groupId: groupId,
        eventId: event.id,
        timestamp: event.created_at,
        relay: relay.url
      }
    });
  }
}

// Handle Kind 444 (welcome message / invite)
async function handleWelcomeMessage(event, relay) {
  // Extract recipient pubkey from p-tag
  const pTag = event.tags.find(tag => tag[0] === 'p');
  if (!pTag || !pTag[1]) return;

  const recipientPubkey = pTag[1];

  logger.debug(`💌 Welcome message for ${recipientPubkey.substring(0, 16)}... from ${relay.url}`);

  // Check if this user is subscribed
  const userData = subscriptions.get(recipientPubkey);
  if (!userData) {
    logger.debug(`  User not subscribed, skipping`);
    return;
  }

  // Check if we already sent a notification for this event to this user
  const notificationKey = `${event.id}:${recipientPubkey}`;
  if (sentNotifications.has(notificationKey)) {
    const firstRelay = sentNotifications.get(notificationKey).relay;
    logger.debug(`  ⏭️  Skipping duplicate (already sent via ${firstRelay})`);
    return;
  }

  // Record that we're sending this notification
  sentNotifications.set(notificationKey, {
    relay: relay.url,
    timestamp: Date.now()
  });

  // Format timestamp
  const timestamp = new Date(event.created_at * 1000);
  const timeStr = timestamp.toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit' });

  // Extract relay domain
  const relayDomain = relay.url.replace('wss://', '').replace('ws://', '').split('/')[0];

  // Send push notification
  await sendPushNotification(recipientPubkey, userData.subscription, {
    title: 'New group invitation',
    body: `You've been invited to join a new group\nTime: ${timeStr}\nRelay: ${relayDomain}`,
    tag: `welcome-${event.id}`,
    data: {
      eventId: event.id,
      timestamp: event.created_at,
      relay: relay.url
    }
  });
}

// Send push notification with error handling
async function sendPushNotification(pubkey, subscription, payload) {
  try {
    await webpush.sendNotification(subscription, JSON.stringify(payload));
    logger.debug(`✅ Push sent to ${pubkey.substring(0, 16)}...`);
  } catch (err) {
    logger.error(`❌ Failed to send push to ${pubkey.substring(0, 16)}...:`, err.message);

    // Remove invalid subscriptions
    if (err.statusCode === 410) {
      logger.debug(`🗑️  Removing expired subscription for ${pubkey.substring(0, 16)}...`);
      subscriptions.delete(pubkey);
      updateAllRelaySubscriptions();
    }
  }
}

// Update subscriptions on all relays
function updateAllRelaySubscriptions() {
  for (const relay of relayConnections.values()) {
    updateRelaySubscriptions(relay);
  }
}

// API Routes

// Get VAPID public key
app.get('/vapid-public-key', (req, res) => {
  res.json({ publicKey: VAPID_PUBLIC_KEY });
});

// Subscribe to push notifications
app.post('/subscribe', (req, res) => {
  const { pubkey, subscription, groupIds, relays } = req.body;

  if (!pubkey || !subscription || !groupIds) {
    return res.status(400).json({ error: 'Missing required fields: pubkey, subscription, groupIds' });
  }

  if (!relays || !Array.isArray(relays) || relays.length === 0) {
    return res.status(400).json({ error: 'Missing or invalid relays array' });
  }

  // Store subscription with relays
  subscriptions.set(pubkey, {
    subscription,
    groupIds,
    relays,
    subscriptionTimestamp: Date.now()
  });

  logger.debug(`📝 New subscription from ${pubkey.substring(0, 16)}... for ${groupIds.length} groups on ${relays.length} relays`);

  // Connect to relays if needed
  relays.forEach(relayUrl => {
    if (!relayConnections.has(relayUrl)) {
      connectToRelay(relayUrl);
    }
  });

  // Update relay subscriptions
  updateAllRelaySubscriptions();

  res.json({ success: true, message: 'Subscription registered' });
});

// Unsubscribe from push notifications
app.post('/unsubscribe', (req, res) => {
  const { pubkey } = req.body;

  if (!pubkey) {
    return res.status(400).json({ error: 'Missing pubkey' });
  }

  subscriptions.delete(pubkey);
  logger.debug(`🗑️  Unsubscribed ${pubkey.substring(0, 16)}...`);

  // Update relay subscriptions
  updateAllRelaySubscriptions();

  res.json({ success: true, message: 'Unsubscribed' });
});

// Health check
app.get('/health', (req, res) => {
  const connectedRelays = Array.from(relayConnections.values())
    .filter(r => r.connected)
    .map(r => r.url);

  res.json({
    status: 'ok',
    subscriptions: subscriptions.size,
    relays: {
      connected: connectedRelays,
      total: relayConnections.size
    }
  });
});

// Stats endpoint
app.get('/stats', (req, res) => {
  res.json({
    totalSubscriptions: subscriptions.size,
    relays: Array.from(relayConnections.values()).map(r => ({
      url: r.url,
      connected: r.connected
    }))
  });
});

// Start server
app.listen(PORT, () => {
  logger.debug('='.repeat(60));
  logger.debug('🚀 MDK Ecash Push Server');
  logger.debug('='.repeat(60));
  logger.debug(`📍 Listening on port ${PORT}`);
  logger.debug(`🔑 VAPID public key: ${VAPID_PUBLIC_KEY.substring(0, 20)}...`);
  logger.debug('='.repeat(60));
  logger.debug(`📡 Waiting for client subscriptions to connect to relays...`);
  logger.debug('='.repeat(60));

  logger.debug(`\n✅ Server ready! API endpoints:`);
  logger.debug(`   GET  /vapid-public-key - Get public key for client`);
  logger.debug(`   POST /subscribe        - Register push subscription (includes relays)`);
  logger.debug(`   POST /unsubscribe      - Remove push subscription`);
  logger.debug(`   GET  /health           - Health check`);
  logger.debug(`   GET  /stats            - Server statistics`);
  logger.debug('');
});

// Graceful shutdown
process.on('SIGTERM', () => {
  logger.debug('\n🛑 Shutting down gracefully...');

  // Close all relay connections
  for (const relay of relayConnections.values()) {
    relay.ws.close();
  }

  process.exit(0);
});
