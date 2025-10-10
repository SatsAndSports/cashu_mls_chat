// Service Worker for MDK Ecash PWA
const CACHE_NAME = 'mdk-ecash-v1';
const urlsToCache = [
  '/',
  '/index.html',
  '/pkg/mdk_ecash_web_bg.wasm',
  '/pkg/mdk_ecash_web.js',
  'https://unpkg.com/@zxing/library@latest/umd/index.min.js',
  'https://cdnjs.cloudflare.com/ajax/libs/qrcodejs/1.0.0/qrcode.min.js'
];

// Install service worker and cache resources
self.addEventListener('install', event => {
  console.log('[Service Worker] Installing...');
  event.waitUntil(
    caches.open(CACHE_NAME)
      .then(cache => {
        console.log('[Service Worker] Caching app shell');
        return cache.addAll(urlsToCache);
      })
      .catch(err => {
        console.error('[Service Worker] Cache failed:', err);
      })
  );
  // Don't auto-skip waiting - let the user decide via update prompt
});

// Listen for SKIP_WAITING message from client
self.addEventListener('message', event => {
  if (event.data && event.data.type === 'SKIP_WAITING') {
    self.skipWaiting();
  }
});

// Handle notification clicks
self.addEventListener('notificationclick', event => {
  console.log('[Service Worker] Notification clicked:', event.notification.tag);
  event.notification.close();

  // Get the notification data
  const data = event.notification.data;

  // Determine the URL to open
  let targetUrl = '/';
  if (data && data.groupId && data.groupName) {
    // Format: #chat:groupId:groupName
    targetUrl = `/#chat:${data.groupId}:${encodeURIComponent(data.groupName)}`;
  }

  // Open or focus the app window
  event.waitUntil(
    clients.matchAll({ type: 'window', includeUncontrolled: true }).then(windowClients => {
      // Check if there's already a window open
      for (let client of windowClients) {
        if ('focus' in client) {
          client.focus();
          // Navigate to the target URL
          client.postMessage({
            type: 'NAVIGATE',
            url: targetUrl
          });
          return;
        }
      }
      // If no window is open, open a new one
      if (clients.openWindow) {
        return clients.openWindow(targetUrl);
      }
    })
  );
});

// Activate service worker and clean up old caches
self.addEventListener('activate', event => {
  console.log('[Service Worker] Activating...');
  event.waitUntil(
    caches.keys().then(cacheNames => {
      return Promise.all(
        cacheNames.map(cacheName => {
          if (cacheName !== CACHE_NAME) {
            console.log('[Service Worker] Deleting old cache:', cacheName);
            return caches.delete(cacheName);
          }
        })
      );
    })
  );
  // Take control of all pages immediately
  return self.clients.claim();
});

// Intercept fetch requests
self.addEventListener('fetch', event => {
  // Skip non-GET requests
  if (event.request.method !== 'GET') return;

  // Skip chrome-extension and other non-http(s) requests
  if (!event.request.url.startsWith('http')) return;

  event.respondWith(
    caches.match(event.request)
      .then(response => {
        // Cache hit - return response from cache
        if (response) {
          console.log('[Service Worker] Serving from cache:', event.request.url);
          return response;
        }

        // Not in cache - fetch from network
        console.log('[Service Worker] Fetching from network:', event.request.url);
        return fetch(event.request).then(response => {
          // Don't cache if not a valid response
          if (!response || response.status !== 200 || response.type === 'error') {
            return response;
          }

          // Clone the response (can only be consumed once)
          const responseToCache = response.clone();

          // Cache the fetched resource
          caches.open(CACHE_NAME).then(cache => {
            cache.put(event.request, responseToCache);
          });

          return response;
        });
      })
      .catch(err => {
        console.error('[Service Worker] Fetch failed:', err);
        // Return a fallback page when offline
        return new Response('Offline - please check your connection', {
          status: 503,
          statusText: 'Service Unavailable',
          headers: new Headers({
            'Content-Type': 'text/plain'
          })
        });
      })
  );
});
