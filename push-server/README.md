# MDK Ecash Push Server

Push notification server for MDK Ecash web client. Connects to Nostr relays and sends web push notifications to subscribed clients.

## Features

- ✅ Connects to Nostr relays
- ✅ Monitors group messages (Kind 445 events)
- ✅ Sends web push notifications to subscribers
- ✅ Automatic reconnection to relays
- ✅ Handles multiple concurrent subscriptions
- ✅ Removes expired push subscriptions automatically

## Setup

### 1. Install Dependencies

```bash
cd push-server
npm install
```

### 2. Generate VAPID Keys

VAPID keys are used to authenticate your push server with browser push services.

```bash
npm run generate-vapid
```

This will output something like:
```
VAPID_PUBLIC_KEY=BHxS7Wg...
VAPID_PRIVATE_KEY=kQ3bV...
VAPID_SUBJECT=mailto:your-email@example.com
```

### 3. Configure Environment

Create a `.env` file:

```bash
cp .env.example .env
nano .env
```

Add your VAPID keys and configuration:

```env
PORT=3000
VAPID_PUBLIC_KEY=BHxS7Wg...
VAPID_PRIVATE_KEY=kQ3bV...
VAPID_SUBJECT=mailto:your-email@example.com
NOSTR_RELAYS=wss://relay.damus.io,wss://relay.primal.net,wss://nos.lol
```

### 4. Run the Server

Development:
```bash
npm start
```

Production (with PM2):
```bash
npm install -g pm2
pm2 start server.js --name mdk-push
pm2 save
pm2 startup  # Follow instructions to start on boot
```

## API Endpoints

### `GET /vapid-public-key`
Returns the VAPID public key for client-side subscription.

**Response:**
```json
{
  "publicKey": "BHxS7Wg..."
}
```

### `POST /subscribe`
Register a new push subscription.

**Request:**
```json
{
  "pubkey": "user_nostr_pubkey_hex",
  "subscription": {
    "endpoint": "https://fcm.googleapis.com/...",
    "keys": {
      "p256dh": "...",
      "auth": "..."
    }
  },
  "groupIds": ["group_id_1", "group_id_2"]
}
```

**Response:**
```json
{
  "success": true,
  "message": "Subscription registered"
}
```

### `POST /unsubscribe`
Remove a push subscription.

**Request:**
```json
{
  "pubkey": "user_nostr_pubkey_hex"
}
```

### `GET /health`
Health check endpoint.

**Response:**
```json
{
  "status": "ok",
  "subscriptions": 5,
  "relays": {
    "connected": ["wss://relay.damus.io"],
    "total": 3
  }
}
```

### `GET /stats`
Server statistics.

## Deployment

### Using systemd (Linux)

Create `/etc/systemd/system/mdk-push.service`:

```ini
[Unit]
Description=MDK Ecash Push Server
After=network.target

[Service]
Type=simple
User=www-data
WorkingDirectory=/path/to/mdk_ecash/push-server
Environment=NODE_ENV=production
EnvironmentFile=/path/to/mdk_ecash/push-server/.env
ExecStart=/usr/bin/node server.js
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

Enable and start:
```bash
sudo systemctl enable mdk-push
sudo systemctl start mdk-push
sudo systemctl status mdk-push
```

### Using Docker

```dockerfile
FROM node:16-alpine
WORKDIR /app
COPY package*.json ./
RUN npm ci --only=production
COPY . .
EXPOSE 3000
CMD ["node", "server.js"]
```

Build and run:
```bash
docker build -t mdk-push-server .
docker run -d -p 3000:3000 --env-file .env --name mdk-push mdk-push-server
```

### Reverse Proxy (Nginx)

Add to your nginx config:

```nginx
location /push/ {
    proxy_pass http://localhost:3000/;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection 'upgrade';
    proxy_set_header Host $host;
    proxy_cache_bypass $http_upgrade;
}
```

## Client Integration

The web client needs to be updated to:

1. Fetch VAPID public key from server
2. Subscribe to push notifications
3. Send subscription + group IDs to server

Example client code:
```javascript
// Fetch VAPID key
const response = await fetch('https://your-server.com/push/vapid-public-key');
const { publicKey } = await response.json();

// Subscribe to push
const registration = await navigator.serviceWorker.ready;
const subscription = await registration.pushManager.subscribe({
  userVisibleOnly: true,
  applicationServerKey: publicKey
});

// Send to server
await fetch('https://your-server.com/push/subscribe', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({
    pubkey: userPubkey,
    subscription: subscription,
    groupIds: ['group1', 'group2']
  })
});
```

## Security Considerations

- **VAPID private key**: Keep secret, never commit to git
- **HTTPS required**: Web Push API only works over HTTPS
- **Rate limiting**: Consider adding rate limits to prevent abuse
- **Authentication**: Currently unauthenticated - consider adding API keys
- **Privacy**: Server knows which groups users are in (but not message content)

## Monitoring

View logs:
```bash
# PM2
pm2 logs mdk-push

# systemd
journalctl -u mdk-push -f

# Docker
docker logs -f mdk-push
```

## Troubleshooting

**Notifications not working:**
- Check VAPID keys are correct
- Verify relays are connected: `curl http://localhost:3000/health`
- Check browser console for errors
- Ensure HTTPS is used (required for push API)

**Relay disconnections:**
- Server automatically reconnects after 5 seconds
- Check relay URLs are correct
- Some relays may rate-limit connections

**Memory usage:**
- Current implementation uses in-memory storage
- For production, consider using Redis or database
- Each subscription uses ~1KB of memory

## License

Same as MDK Ecash project.
