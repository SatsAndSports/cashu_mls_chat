# MDK Ecash - Context for Claude

This document provides comprehensive context for Claude (or any AI assistant) working on the MDK Ecash project.

## Project Overview

**MDK Ecash** is a web-based encrypted group messaging application that integrates Bitcoin ecash (Cashu) payments. It combines:
- **MLS (Message Layer Security)** for end-to-end encrypted group messaging
- **Nostr** as the transport/relay layer for messages
- **Cashu** for Bitcoin ecash payments within groups
- **Lightning Network** for on/off-ramping to ecash

## Architecture

### Components

1. **Web Client** (`/web/`)
   - Single-page application (HTML + JavaScript + WebAssembly)
   - Runs entirely in the browser
   - No traditional backend - uses Nostr relays for communication

2. **WASM Module** (`/web/src/`)
   - Rust code compiled to WebAssembly
   - Handles MLS encryption, Nostr protocol, Cashu wallet operations
   - Uses `wasm-bindgen` to expose Rust functions to JavaScript

3. **Push Server** (`/push-server/`)
   - **Optional** Node.js server for background notifications
   - Connects to Nostr relays
   - Sends web push notifications to clients when messages arrive
   - Can be self-hosted on user's VPS

### Data Storage

All data is stored in browser's `localStorage`:
- **Nostr keys** - User's identity (nsec/npub keypair)
- **Wallet state** - Cashu proofs, balances, mint keys
- **MLS state** - Group encryption state, member credentials
- **Profile cache** - Cached Nostr profile metadata (Kind 0 events)

Database implementation: `/web/src/wallet_db.rs` - `HybridWalletDatabase` struct

### Key Technologies

- **MLS (OpenMLS)** - End-to-end encrypted group messaging
- **Nostr** - Decentralized relay network (similar to ActivityPub but simpler)
- **Cashu (CDK)** - Chaumian ecash protocol for Bitcoin
- **WebAssembly** - Rust compiled to run in browser
- **Service Workers** - For PWA offline support and push notifications

## Nostr Integration

### Event Kinds Used

- **Kind 10050** - KeyPackage (MLS public key material for adding to groups)
- **Kind 443** - Welcome message (encrypted invite to join group)
- **Kind 445** - Group message (encrypted MLS application messages)
- **Kind 0** - User metadata (display name, profile)
- **Kind 5** - Deletion event (delete used KeyPackages)

### Group Identification

Each MLS group has TWO identifiers:
- **`mls_group_id`** - Never changes, primary key in storage
- **`nostr_group_id`** - Used in Nostr event tags (`#h` tag), currently stable but has setter

**Current implementation:** `nostr_group_id` is randomly generated once at group creation and remains stable. All messages are tagged with `#h:<nostr_group_id>` for efficient relay filtering.

**Subscription model:** Client subscribes to `{ kinds: [445], '#h': '<nostr_group_id>' }`

### Relay Management

- Default relays configured in code
- User can add/remove relays in Settings
- Messages are published to all configured relays
- Client subscribes to all relays for receiving messages

## Cashu Wallet

### Multi-Mint Design

The wallet supports multiple Cashu mints simultaneously:
- Each mint has separate balance tracking
- "Trusted mints" list stored in localStorage
- Can send/receive tokens from any mint
- Untrusted mint warning modal when receiving from new mint

### Operations

- **Mint** - Create ecash from Lightning invoice
- **Melt** - Pay Lightning invoice with ecash
- **Send** - Create ecash token from balance
- **Receive** - Redeem ecash token into balance
- **P2PK** - Lock tokens to recipient's pubkey (optional)

### In-Chat Payments

Special feature: Send ecash tokens as messages in groups
- Token is sent as an MLS encrypted message
- Appears as "💰 Cashu Token" with redeem button
- Recipient can redeem with one click
- Shows amount and mint info

## UI Structure

### Sections (Single Page App)

1. **Identity & Key Packages** - Manage Nostr keys, view/refresh KeyPackages
2. **Relays** - Configure Nostr relays
3. **Wallet** - View balance, mint/melt, transaction history
4. **Groups** - List groups, create new, join via QR
5. **Settings** - Notifications, push server configuration
6. **Chat** (dynamic) - Opens when clicking a group

### Navigation

- Hash-based routing: `#groups`, `#wallet`, `#chat:groupId:groupName`
- Back/forward browser buttons work
- State preserved in URL for sharing/bookmarking

## Progressive Web App (PWA)

### Files

- **`/web/manifest.json`** - PWA manifest (app name, icons, display mode)
- **`/web/sw.js`** - Service worker (caching, push notifications)
- **`/web/icon-192.png`** and **`icon-512.png`** - App icons (must be created)
- **`/web/generate-icons.html`** - Tool to generate icons in browser

### Features

- ✅ Installable on mobile/desktop
- ✅ Offline caching (UI shell + WASM module)
- ✅ Update detection with user prompt
- ✅ Works without network (but can't send/receive messages)

### Service Worker Update Flow

1. Browser checks for new `sw.js` on page load
2. If changed, new service worker installs in background
3. User sees update banner: "A new version is available!"
4. User clicks "Update Now" → service worker activates → page reloads
5. **Important:** Must bump `CACHE_NAME` (e.g., `mdk-ecash-v2` → `v3`) on every deploy

## Push Notifications System

### Overview

MDK Ecash has a **two-tier notification system**:

1. **In-App Notifications** - Works immediately, no setup, all browsers
2. **Background Push Notifications** - Requires push server, Chrome/Edge only on Linux

### The Web Push Problem

**Core issue:** Web browsers cannot maintain persistent connections when tabs are closed.

**Standard web push architecture requires:**
```
Server → Browser Push Service (Google/Mozilla/Apple) → Service Worker → Notification
```

**What MDK Ecash needs:**
```
Nostr Relay → [something] → Browser Push Service → Service Worker → Notification
```

**The gap:** Need a server that:
- Stays connected to Nostr relays 24/7
- Monitors messages for user's groups
- Triggers web push when messages arrive
- Can't be the Nostr relay itself (relays don't support web push protocol)

### Why Not Other Solutions?

**Service Worker WebSocket (Hybrid Approach):**
- ❌ Service workers can be killed by browser anytime (30-60 seconds typical)
- ❌ No guaranteed keep-alive
- ❌ Battery drain
- ❌ Unreliable

**Periodic Background Sync:**
- ❌ Not supported in Firefox
- ❌ Minimum interval ~12 hours in practice (browser decides)
- ❌ Not real-time

**Browser Extension:**
- ❌ Separate installation
- ❌ Not a web app
- ❌ Chrome Web Store approval needed

**Why can't Nostr relays be push servers?**
- Web push requires VAPID keys (cryptographic signature from YOUR server)
- Push endpoints should not be public on Nostr (privacy)
- No relay implements web push protocol (would need new NIP)
- Would need to convince relay operators to add this feature

### Architecture: In-App Notifications

**How it works:**
```
Nostr Relay → WebSocket → Browser Tab (open) → JavaScript → Notification API
```

**Implementation:**
- `addMessageToChat()` function checks if message is from someone else
- Calls `showNotification()` with author, group name, message preview
- Service worker shows notification (or fallback to `new Notification()`)
- Click notification → opens app to that chat

**Code locations:**
- Subscription setup: `subscribeToMessages()` in index.html
- Message handler: `addMessageToChat()` - line ~2200
- Welcome handler: `initializeWelcomeSubscription()` - line ~3034
- Notification display: `showNotification()` - line ~1611

**Limitations:**
- Only works when tab is open (can be minimized/background)
- If all tabs closed, no notifications

**Browser support:**
- ✅ Chrome/Edge/Firefox/Safari - all work
- ✅ Desktop and mobile
- ✅ iOS Safari (when PWA installed)

### Architecture: Background Push Notifications

**How it works:**
```
Nostr Relay → Push Server → Browser Push Service → Service Worker → Notification
```

**Push Server responsibilities:**
1. Maintain WebSocket connections to Nostr relays
2. Store client push subscriptions (endpoint URLs + user's group IDs)
3. Monitor Nostr events (Kind 445) for subscribed groups
4. Send web push to relevant clients when messages arrive
5. Handle subscription management (add/remove)

**Client responsibilities:**
1. Subscribe to browser push (get endpoint URL)
2. Send endpoint + group IDs + user pubkey to push server
3. Handle push events in service worker
4. Update subscriptions when joining/leaving groups

**Web Push Protocol:**
- Uses VAPID (Voluntary Application Server Identification)
- VAPID keys: Public key (shared with clients) + Private key (server only)
- Push Service providers: Google (FCM), Mozilla (autopush), Apple (APNs)
- Server signs push messages with private key
- Browser verifies with public key

### Push Server Implementation

**Location:** `/push-server/`

**Key files:**
- `server.js` - Main server (Express + WebSocket + web-push library)
- `package.json` - Dependencies
- `generate-vapid.js` - One-time key generation
- `.env` - Configuration (VAPID keys, relays, port)

**Architecture (server.js):**
```javascript
// Data structure
subscriptions = Map<userPubkey, {
    subscription: PushSubscription,  // Browser endpoint
    groupIds: string[],              // Groups user is in
    subscriptionTimestamp: number
}>

relayConnections = Map<relayUrl, {
    ws: WebSocket,
    connected: boolean
}>

// Flow
1. Client POSTs to /subscribe with { pubkey, subscription, groupIds }
2. Server stores in subscriptions Map
3. Server updates Nostr relay filters to include all groupIds
4. When Nostr event arrives → check which users care → send web push
5. If push fails with 410 → remove expired subscription
```

**API Endpoints:**
- `GET /vapid-public-key` - Returns public key for client subscription
- `POST /subscribe` - Register client subscription
- `POST /unsubscribe` - Remove client subscription
- `GET /health` - Health check (connected relays, subscription count)
- `GET /stats` - Statistics

**Nostr Integration:**
```javascript
// Server subscribes to all groups
['REQ', 'mdk-push', {
    kinds: [445],
    '#h': [groupId1, groupId2, ...]
}]

// On EVENT received
if (event is in user's groups && event.pubkey !== user.pubkey) {
    webpush.sendNotification(user.subscription, {
        title: 'New message',
        body: messagePreview,
        tag: `message-${groupId}`,
        data: { groupId, eventId, timestamp }
    })
}
```

**Subscription Management:**
- Subscriptions stored in-memory (lost on restart)
- For production: Use Redis or database
- Expired subscriptions removed on 410 error (Gone)
- Clients resubscribe on app startup if notifications enabled

**Reconnection Logic:**
- If relay disconnects → auto-reconnect after 5 seconds
- Resubscribe to filters on reconnect
- Logs all connection events

### Client-Side Push Integration

**Configuration UI:**
- Settings section has "Push Server URL" input field
- User enters `https://their-domain.com/push`
- Saved to `localStorage.push_server_url`
- Optional - leave empty for in-app notifications only

**Subscription Flow:**
```javascript
// 1. Get VAPID public key from server
fetch(PUSH_SERVER_URL + '/vapid-public-key')

// 2. Subscribe to browser push
registration.pushManager.subscribe({
    userVisibleOnly: true,
    applicationServerKey: vapidPublicKey  // Converted to Uint8Array
})

// 3. Get user's groups
const groups = await get_groups()
const groupIds = groups.map(g => g.nostr_group_id)

// 4. Send to push server
fetch(PUSH_SERVER_URL + '/subscribe', {
    method: 'POST',
    body: JSON.stringify({
        pubkey: userPubkey,
        subscription: pushSubscription,
        groupIds: groupIds
    })
})
```

**Code locations:**
- Push server URL config: Lines 511-525 (Settings section HTML)
- `subscribeToPushServer()` function: Line ~1405
- `savePushServerUrl()` function: Line ~1569
- `toggleNotifications()` integration: Line ~1464
- `initializeNotifications()`: Line ~1395

**Service Worker Push Handler:**
```javascript
// sw.js line ~36
self.addEventListener('push', event => {
    const data = event.data.json()
    self.registration.showNotification(data.title, {
        body: data.body,
        icon: '/icon-192.png',
        tag: data.tag,
        data: data.data  // Contains groupId, groupName for click handling
    })
})
```

**Notification Click Handling:**
```javascript
// sw.js line ~65
self.addEventListener('notificationclick', event => {
    // Extract groupId and groupName from notification data
    const targetUrl = `/#chat:${groupId}:${encodeURIComponent(groupName)}`

    // Focus existing window or open new
    clients.matchAll() → focus() or openWindow(targetUrl)

    // Send navigation message to client
    client.postMessage({ type: 'NAVIGATE', url: targetUrl })
})
```

### Push Server Setup (VPS)

**Prerequisites:**
- Node.js 16+
- HTTPS domain (required for web push)
- Port 3000 available (or configure different port)
- Access to configure nginx/reverse proxy

**Installation Steps:**

1. **Install dependencies:**
```bash
cd /path/to/mdk_ecash/push-server
npm install
```

2. **Generate VAPID keys:**
```bash
npm run generate-vapid
```
Output:
```
VAPID_PUBLIC_KEY=BHxS7Wg...
VAPID_PRIVATE_KEY=kQ3bV...
```

3. **Configure environment:**
```bash
cp .env.example .env
nano .env
```
```env
PORT=3000
VAPID_PUBLIC_KEY=BHxS7Wg...
VAPID_PRIVATE_KEY=kQ3bV...
VAPID_SUBJECT=mailto:your-email@example.com
NOSTR_RELAYS=wss://relay.damus.io,wss://relay.primal.net,wss://nos.lol
```

4. **Run with PM2 (recommended):**
```bash
npm install -g pm2
pm2 start server.js --name mdk-push
pm2 save
pm2 startup  # Run the command it outputs
```

5. **Or systemd:**
Create `/etc/systemd/system/mdk-push.service`:
```ini
[Unit]
Description=MDK Ecash Push Server
After=network.target

[Service]
Type=simple
User=www-data
WorkingDirectory=/path/to/mdk_ecash/push-server
EnvironmentFile=/path/to/mdk_ecash/push-server/.env
ExecStart=/usr/bin/node server.js
Restart=on-failure

[Install]
WantedBy=multi-user.target
```
```bash
sudo systemctl enable mdk-push
sudo systemctl start mdk-push
sudo systemctl status mdk-push
```

6. **Configure nginx reverse proxy:**
```nginx
location /push/ {
    proxy_pass http://localhost:3000/;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection 'upgrade';
    proxy_set_header Host $host;
    proxy_cache_bypass $http_upgrade;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
}
```
```bash
sudo nginx -t
sudo systemctl reload nginx
```

7. **Test server:**
```bash
# Health check
curl https://your-domain.com/push/health

# Should return:
# {"status":"ok","subscriptions":0,"relays":{"connected":[...],"total":3}}

# Get public key
curl https://your-domain.com/push/vapid-public-key
```

8. **Configure web client:**
- Open app → Settings
- Enter: `https://your-domain.com/push`
- Click Save
- Enable notifications

9. **Test notifications:**
- Close all app tabs
- Have someone send a message
- Should receive notification within seconds

### Monitoring and Debugging

**View logs:**
```bash
# PM2
pm2 logs mdk-push
pm2 logs mdk-push --lines 100

# systemd
journalctl -u mdk-push -f
journalctl -u mdk-push --since "1 hour ago"
```

**Check subscriptions:**
```bash
curl https://your-domain.com/push/stats
```

**Common issues:**

1. **No notifications arriving:**
   - Check if server is connected to relays: `/health` endpoint
   - Check browser granted notification permission
   - Check console for subscription errors
   - Verify HTTPS (required for push API)

2. **Server keeps disconnecting:**
   - Check relay URLs in .env
   - Some relays rate-limit connections
   - Try different relays

3. **Subscription fails:**
   - Check VAPID keys are correct
   - Verify push server URL is accessible from browser
   - Check CORS if domains differ

4. **Notifications don't open correct chat:**
   - Check notification data includes groupId and groupName
   - Verify service worker click handler is registered
   - Check browser console for navigation errors

**Performance monitoring:**
```bash
# Memory usage
pm2 monit

# Or
ps aux | grep node

# Typical usage: ~50MB base + ~1KB per subscription
# With 100 users: ~50MB + 100KB = ~50.1MB
```

**Resource limits:**
- Each WebSocket connection: ~1KB
- Each subscription: ~1KB
- Typical VPS can handle thousands of users
- Consider Redis for >1000 concurrent subscriptions

### Browser Support Matrix

| Platform | Browser | In-App | Background Push |
|----------|---------|--------|-----------------|
| Linux | Chrome/Edge | ✅ | ✅ |
| Linux | Firefox | ✅ | ❌ (platform limitation) |
| Windows | Chrome/Edge | ✅ | ✅ |
| Windows | Firefox | ✅ | ✅ |
| macOS | Chrome/Edge | ✅ | ✅ |
| macOS | Firefox | ✅ | ✅ |
| macOS | Safari | ✅ | ✅ |
| iOS | Safari | ✅ | ✅ (iOS 16.4+, PWA only) |
| iOS | Chrome | ✅ | ❌ (uses Safari engine) |
| Android | Chrome | ✅ | ✅ |
| Android | Firefox | ✅ | ✅ |

**Firefox on Linux limitation:**
- Firefox doesn't implement background push properly on Linux
- Service worker can't be woken when browser is closed
- Works fine on Windows/Mac
- User should use Chrome/Edge on Linux for background push

### Security and Privacy

**VAPID Keys:**
- Private key must remain secret (never commit to git)
- Public key is shared with all clients
- Keys are cryptographic proof that push is from your server
- `.gitignore` includes `.env` to prevent accidental commit

**Push Server Privacy:**
- Server knows: user pubkey, group IDs, push endpoint
- Server does NOT know: message content (encrypted by MLS)
- Server does NOT know: user identity (pubkey is pseudonymous)
- Self-hosting option for privacy-focused users

**Push Endpoint Privacy:**
- Endpoint is a unique URL from Google/Mozilla/Apple
- Tied to browser + device combination
- Can be revoked by user (browser settings)
- Not shared with anyone except push server

**Data Retention:**
- In-memory storage (lost on restart)
- No persistent logging of messages or users
- Only stores subscriptions while active
- Expired subscriptions automatically removed

**HTTPS Requirement:**
- Web Push API only works over HTTPS
- Required for service worker registration
- Can use Let's Encrypt for free SSL certificates

### Future Improvements

**Not yet implemented:**

1. **Subscription Persistence:**
   - Current: In-memory (lost on server restart)
   - Improvement: Store in Redis or database
   - Clients auto-resubscribe on next startup

2. **Group Update Notifications:**
   - Current: Only notify on new messages
   - Improvement: Notify on member add/remove, group rename

3. **Rate Limiting:**
   - Current: No rate limits
   - Improvement: Limit subscriptions per IP, push frequency

4. **Authentication:**
   - Current: No API authentication
   - Improvement: Nostr event-based auth (NIP-98)

5. **Multi-Device Support:**
   - Current: One subscription per pubkey
   - Improvement: Array of subscriptions per user

6. **Notification Preferences:**
   - Current: All messages trigger notifications
   - Improvement: Mute specific groups, DND mode

7. **Relay-Based Push (Future NIP):**
   - Propose NIP for relays to support web push directly
   - Would eliminate need for separate push server
   - Requires relay operator adoption (years away)

8. **Push Server Clustering:**
   - Current: Single server instance
   - Improvement: Load balanced, shared Redis backend

## QR Code Scanning

### Current Implementation

Uses **ZXing library** for QR code scanning:
- Library: `https://unpkg.com/@zxing/library@latest/umd/index.min.js`
- Better recognition at angles than previous html5-qrcode
- Replaced in earlier session for reliability

### Three Scanners

1. **Lightning Invoice Scanner** - `showQRScanner()` - Scans invoices, strips `lightning:` prefix if present
2. **Create Group Scanner** - `showGroupQRScanner()` - Scans group invite codes
3. **Invite Member Scanner** - `showInviteMemberQRScanner()` - Scans npub to invite

### Error Handling

- Checks if scanner already running before starting new one
- Properly stops video streams on modal close
- Handles `AbortError: Starting videoinput failed` by cleaning up old scanners

### Code Locations

- Lightning scanner: Line ~3089
- Group scanner: Line ~3182
- Invite scanner: Line ~3234
- ZXing script tag: Line 19

## Performance Optimizations

### Profile Metadata Caching

**Problem:** Fetching Kind 0 events (display names) for every message author was blocking UI and causing slow page loads (5-10 seconds per profile).

**Solution:** Non-blocking profile fetches with aggressive caching

**Implementation:**
```javascript
// Cache structure
profileCache = {
    npub: {
        cached_at: timestamp,
        data: null | { display_name: "Name", ... }
    }
}

// Render flow
1. Show truncated npub immediately (first 16 chars)
2. Fetch display name in background
3. Update DOM when ready
4. Cache for 24 hours
5. Cache "no profile found" as null to avoid repeated queries
```

**Code locations:**
- Cache management: Line ~2512
- `getDisplayName()`: Line ~2565
- Non-blocking message render: Line ~2200 (`addMessageToChat`)
- Non-blocking message load: Line ~2097 (`loadMessages`)

**Cache invalidation:** On page refresh via `invalidateCacheOnRefresh()` - uses session storage to detect refreshes

### Modal Scrolling

All modals are scrollable to prevent content cutoff:
- Outer modal: `overflow-y: auto`
- Inner content: `max-height: calc(100vh - 100px); overflow-y: auto;`
- Applied to 6+ modals: receive-lightning, npub-qr, create-group, invite-member, pay-lightning, invite-details

## Known Issues

### MLS Commit Conflicts (Not Yet Handled)

**Problem:** When two users perform epoch-changing operations simultaneously (e.g., both adding a member), only one commit succeeds.

**Example:**
- Alice and Bob both try to add Carol at epoch 5
- Both create commits: 5→6 (different commits)
- Alice publishes first → accepted
- Bob publishes second → rejected (already at epoch 6)
- Bob's client doesn't detect the conflict or retry

**Current behavior:**
- Error logged but not handled
- Bob's state may diverge from group

**Fix needed:**
- Detect commit conflicts (epoch mismatch errors)
- Re-sync state by processing missed commits
- Retry operation if still valid
- Show user-friendly error if invalid

**Deferred:** User decided to think about this more in the future

### Nostr Group ID Stability

**Current:** `nostr_group_id` is randomly generated once and stable. A setter exists but is never called in production.

**Potential issue:** If `nostr_group_id` changes in future:
- Old messages have `#h:<old_id>`
- New messages have `#h:<new_id>`
- Current subscription only sees current ID
- Would need to track ID history

**Recommendation:** Clarify whether `nostr_group_id` is immutable. If yes, remove setter. If no, implement multi-ID subscription.

## Development Workflow

### Building

```bash
cd web

# Development (faster, no optimization)
wasm-pack build --target web --dev

# Production (slower, optimized)
wasm-pack build --target web
```

**Important:** Always use `--dev` during development to avoid wasm-opt timeouts (can take 5+ minutes).

### Running Locally

```bash
cd web
python3 -m http.server 4450
```

Open http://localhost:4450

### Testing

**Typical test scenario:**
1. Open in two browser windows (or devices)
2. Create group in window 1
3. Generate KeyPackage in window 2
4. Copy QR code from window 2
5. Scan QR code in window 1 to invite
6. Window 2 automatically joins (Welcome message)
7. Send messages back and forth

**Testing notifications:**
1. Enable in Settings
2. Click "Send Test Notification"
3. Send message from another device/window
4. Should see notification immediately

**Testing background push:**
1. Set up push server on VPS (see above)
2. Configure URL in Settings
3. Enable notifications
4. Close all tabs
5. Send message from another device
6. Should receive notification within seconds

### Deployment

1. Build for production: `wasm-pack build --target web`
2. Copy entire `/web` directory to web server
3. Ensure HTTPS is configured (required for PWA and push)
4. Update service worker cache version in `sw.js` (increment `CACHE_NAME`)
5. If using push server: Update push server URL in client Settings

**When deploying updates:**
- Always increment `CACHE_NAME` in `sw.js` (e.g., `mdk-ecash-v2` → `mdk-ecash-v3`)
- Users will see update banner and can choose to update immediately
- Service worker automatically clears old caches

## Code Organization

### Web Client (`/web/index.html`)

Single file application (4600+ lines):
- Lines 1-350: HTML structure and CSS
- Lines 351-1260: Modals and UI elements
- Lines 1261-4600: JavaScript functions

**Key function categories:**
- WASM initialization: `loadWasm()` - Line 1303
- Identity: `displayNpub()`, `generateKey()`, `importKey()`
- Groups: `refreshGroups()`, `openChat()`, `createGroup()`
- Chat: `loadMessages()`, `addMessageToChat()`, `sendChatMessage()`
- Wallet: `refreshMintBalances()`, `showSendModal()`, `receiveToken()`
- Notifications: `toggleNotifications()`, `showNotification()`, `subscribeToPushServer()`
- QR Scanning: `showQRScanner()`, `showGroupQRScanner()`, `showInviteMemberQRScanner()`
- Settings: `loadSettings()`, `savePushServerUrl()`

### WASM Module (`/web/src/`)

**Entry point:** `lib.rs`

**Key modules:**
- `wallet_db.rs` - Database implementation for Cashu wallet
- (Other Rust modules compiled to WASM)

**WASM functions exposed to JavaScript:**
- `init()` - Initialize WASM module
- `get_pubkey_hex()` - Get user's Nostr pubkey
- `get_groups()` - Get all MLS groups
- `send_message_to_group()` - Encrypt and send message
- `subscribe_to_messages()` - Subscribe to group messages from relays
- `subscribe_to_welcome_messages()` - Subscribe to Welcome messages
- `add_relay()`, `remove_relay()` - Manage relays
- Plus Cashu wallet functions

### Service Worker (`/web/sw.js`)

Lines 1-35: Install and activation
Lines 36-62: Push event handler
Lines 65-100: Notification click handler
Lines 103+: Fetch event handler (caching)

**Current cache version:** `mdk-ecash-v2`

### Push Server (`/push-server/server.js`)

Lines 1-30: Configuration and setup
Lines 32-70: Relay connection management
Lines 72-120: Nostr message handling
Lines 122-180: Express API routes
Lines 182+: Server startup and cleanup

## Documentation Files

- `README.md` - Main project overview
- `NOTIFICATIONS.md` - User guide for notifications
- `push-server/README.md` - Push server technical documentation
- `web/pkg/README.md` - WASM build instructions
- `CLAUDE.md` - This file (context for AI assistants)

## Common User Tasks

### Setting up push notifications on VPS

See detailed instructions in "Push Server Setup (VPS)" section above.

**Quick checklist:**
- [ ] Install Node.js 16+
- [ ] Clone repo to VPS
- [ ] `cd push-server && npm install`
- [ ] `npm run generate-vapid` → save keys to .env
- [ ] Configure .env (keys, relays, port)
- [ ] Run with PM2 or systemd
- [ ] Configure nginx reverse proxy with HTTPS
- [ ] Test: `curl https://domain.com/push/health`
- [ ] Configure URL in web client Settings
- [ ] Enable notifications in client

### Debugging notification issues

1. **Check browser permission:**
   - Chrome: Settings → Privacy → Notifications
   - Firefox: Preferences → Privacy → Permissions → Notifications
   - Safari: Preferences → Websites → Notifications

2. **Check service worker:**
   - Open DevTools → Application → Service Workers
   - Should show "mdk-ecash-v2" or higher
   - Status should be "activated and is running"

3. **Check push subscription:**
   - Application → Storage → IndexedDB → (none for this app)
   - Or check localStorage for "notifications_enabled" and "push_server_url"

4. **Check push server:**
   - SSH to VPS
   - `pm2 status` or `systemctl status mdk-push`
   - `pm2 logs mdk-push` for errors
   - `curl localhost:3000/health` to test locally

5. **Check browser console:**
   - Look for errors from `subscribeToPushServer()`
   - Check network tab for failed requests to push server

### Generating icons for PWA

Option 1: Use browser-based generator
1. Open http://localhost:4450/generate-icons.html
2. Click "Download icon-192.png"
3. Click "Download icon-512.png"
4. Save both to `/web/` directory

Option 2: Use existing SVG
1. Edit `/web/icon.svg` as needed
2. Use Inkscape, ImageMagick, or online converter
3. Export to 192x192 and 512x512 PNG
4. Save to `/web/` directory

## Git Workflow

**Important files to never commit:**
- `/push-server/.env` (contains VAPID private key)
- `/push-server/node_modules/`
- `/web/pkg/` (generated WASM build output - excluded from git per user's request)

**`.gitignore` includes:**
```
push-server/.env
push-server/node_modules/
push-server/*.log
web/pkg/
target/
wallets/
```

## Important Notes for Future Sessions

1. **Service worker updates:** Always increment `CACHE_NAME` when deploying
2. **Push server security:** VAPID private key is sensitive, never commit
3. **WASM builds:** Use `--dev` flag during development to avoid timeouts
4. **Profile caching:** Cache is aggressive (24 hours) to avoid relay spam
5. **Nostr group ID:** Currently stable but has setter - clarify mutability if changing
6. **MLS commit conflicts:** Known issue, not yet handled, user wants to think about it
7. **Push server storage:** In-memory only, consider Redis for production
8. **Browser support:** Firefox on Linux doesn't support background push
9. **HTTPS required:** For PWA, service workers, and push notifications
10. **Testing setup:** Requires two browsers/devices for full message flow testing

## Troubleshooting Common Issues

### WASM build timeout
```bash
# Use --dev flag
wasm-pack build --target web --dev
```

### Service worker not updating
```bash
# Increment version in sw.js
const CACHE_NAME = 'mdk-ecash-v3';  # was v2
```

### Push server won't start
```bash
# Check .env exists and has VAPID keys
cat push-server/.env

# Regenerate keys if needed
cd push-server
npm run generate-vapid
```

### Notifications not working
```bash
# Check browser permission granted
# Check service worker registered
# Check push server running (if using background push)
# Check console for errors
```

### Profile names not loading
```bash
# Check relays are connected
# Check console for timeout errors
# Try clearing localStorage and refreshing
```

## Summary for Quick Context

- **What:** Encrypted group messaging + Bitcoin ecash payments
- **How:** Rust/WASM in browser, Nostr for transport, MLS for encryption, Cashu for payments
- **Storage:** All in browser localStorage (no backend database)
- **Notifications:** Two-tier (in-app + optional push server)
- **PWA:** Installable, offline-capable, update detection
- **Push Server:** Optional Node.js server for background notifications
- **Main Challenge:** Making web push work without persistent connection requires external server

This document should provide comprehensive context for any future work on the project, especially around setting up and troubleshooting the push notification system.
