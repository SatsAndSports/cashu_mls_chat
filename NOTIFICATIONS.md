# Push Notifications Setup Guide

This guide explains how to set up push notifications for MDK Ecash.

## How It Works

MDK Ecash supports two notification modes:

### 1. **In-App Notifications** (Default)
- Works immediately, no setup needed
- Shows notifications when the app is open (tab can be in background)
- Works on all browsers: Firefox, Chrome, Safari
- ✅ Already implemented and working

### 2. **Background Push Notifications** (Optional)
- Shows notifications even when the app/tab is closed
- Requires running a push server on your VPS
- Only works on Chrome/Edge/Chromium (not Firefox on Linux)
- Requires HTTPS

## Quick Start (In-App Only)

1. Open the app
2. Go to **Settings** section
3. Toggle **Enable Notifications** on
4. Grant permission when prompted
5. ✅ Done! You'll get notifications when the app is open

## Full Setup (Background Push)

### Step 1: Copy Push Server Code to Your VPS

```bash
# On your VPS
cd /path/to/mdk_ecash/push-server
```

### Step 2: Generate VAPID Keys

**Option A: Using Docker (no Node.js needed):**
```bash
docker run --rm node:16 bash -c "npm install -g web-push && web-push generate-vapid-keys"
```

**Option B: Using npm (if you have Node.js):**
```bash
npm install
npm run generate-vapid
```

This outputs something like:
```
VAPID_PUBLIC_KEY=BHxS7Wg1234...
VAPID_PRIVATE_KEY=kQ3bV5678...
```

### Step 3: Configure Environment

Create `.env` file:

```bash
cp .env.example .env
nano .env
```

Add your keys:
```env
PORT=3000
VAPID_PUBLIC_KEY=BHxS7Wg1234...
VAPID_PRIVATE_KEY=kQ3bV5678...
VAPID_SUBJECT=mailto:your-email@example.com

# Note: Nostr relays are provided by clients when they subscribe (no need to configure here)
```

### Step 4: Run with Docker (Recommended)

```bash
# Build and start in background
docker-compose up -d --build

# View logs
docker-compose logs -f

# Stop
docker-compose down
```

**Alternative: Run with Node.js directly**

**Development:**
```bash
npm install
npm start
```

**Production (with PM2):**
```bash
npm install -g pm2
pm2 start server.js --name mdk-push
pm2 save
pm2 startup  # Follow instructions
```

### Step 5: Configure Reverse Proxy (Nginx)

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

Reload nginx:
```bash
sudo nginx -t
sudo systemctl reload nginx
```

### Step 6: Configure Web Client

1. Open your MDK Ecash web app
2. Go to **Settings** section
3. Enter your push server URL:
   ```
   https://your-domain.com/push
   ```
4. Click **Save**
5. Toggle **Enable Notifications** on
6. Grant permission

✅ Done! You'll now get notifications even when the app is closed.

## Testing

### Test In-App Notifications

1. Enable notifications in Settings
2. Click "Send Test Notification"
3. You should see a notification

### Test Background Push

1. Configure push server URL
2. Enable notifications
3. Close the app (close all tabs)
4. Have someone send you a message
5. You should get a notification within seconds

### Check Server Status

```bash
# Health check
curl https://your-domain.com/push/health

# Server stats
curl https://your-domain.com/push/stats

# View logs (Docker)
docker-compose logs -f

# View logs (PM2)
pm2 logs mdk-push

# View logs (systemd)
journalctl -u mdk-push -f
```

## Browser Support

| Feature | Chrome | Firefox | Safari |
|---------|--------|---------|--------|
| In-app notifications | ✅ | ✅ | ✅ |
| Background push (Linux) | ✅ | ❌ | N/A |
| Background push (Mac/Win) | ✅ | ✅ | ✅ |
| Background push (iOS) | ✅ | N/A | ✅ (iOS 16.4+, PWA only) |
| Background push (Android) | ✅ | ✅ | N/A |

## Troubleshooting

### Notifications not showing

**Check browser permission:**
- Chrome: Settings → Privacy → Site Settings → Notifications
- Firefox: Preferences → Privacy → Permissions → Notifications
- Look for your site and ensure it's "Allow"

**Check console:**
- Open browser DevTools (F12)
- Look for errors in Console tab
- Check service worker in Application tab

### Background push not working

**Verify HTTPS:**
- Push API only works over HTTPS (or localhost for testing)
- Check your SSL certificate is valid

**Check push server:**
```bash
# Is it running?
curl https://your-domain.com/push/health

# Check logs (Docker)
docker-compose logs -f

# Check logs (PM2)
pm2 logs mdk-push
```

**Verify subscription:**
- Open DevTools → Application → Service Workers
- Check if push subscription exists
- Try unsubscribing and resubscribing

### Server keeps disconnecting from relays

**Check relay URLs:**
- Some relays may rate-limit or block connections
- Try different relays in .env file

**Monitor reconnections:**
- Server auto-reconnects after 5 seconds
- Check logs for repeated disconnections

## Security Considerations

- **VAPID private key**: Keep secret, never commit to git (added to .gitignore)
- **HTTPS required**: Web Push only works over HTTPS
- **Privacy**: Push server knows which groups you're in (but not message content)
- **Rate limiting**: Consider adding rate limits to prevent abuse (not implemented yet)
- **Authentication**: Currently no API authentication - consider adding for production

## Resource Usage

**Push Server:**
- Memory: ~50MB base + ~1KB per active subscription
- CPU: Minimal (mostly idle, spikes on message processing)
- Network: WebSocket connections to Nostr relays
- Storage: In-memory only (uses ~0 disk space)

**Web Client:**
- No additional overhead for background push
- Service worker handles push events efficiently

## Self-Hosting for Privacy

The push server code is open source. Privacy-focused users can:

1. Run their own push server on their own VPS
2. Configure their client to use their own server
3. No third-party sees their subscription data

The server only needs:
- Your VAPID keys (generated locally)
- Access to public Nostr relays (same as client)
- No user authentication or tracking

## Future Improvements

Potential enhancements (not implemented):

- [ ] Redis/database for subscription persistence
- [ ] Rate limiting and abuse prevention
- [ ] API authentication
- [ ] Multi-user support with account management
- [ ] Prometheus metrics for monitoring
- [ ] Relay-based push (NIP proposal)

## Questions?

See the main README or open an issue on GitHub.
