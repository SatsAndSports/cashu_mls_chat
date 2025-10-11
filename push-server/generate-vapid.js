// Generate VAPID keys for web push
// Run once: node generate-vapid.js

const webpush = require('web-push');

const vapidKeys = webpush.generateVAPIDKeys();

console.log('='.repeat(60));
console.log('VAPID Keys Generated');
console.log('='.repeat(60));
console.log('\nAdd these to your .env file:\n');
console.log(`VAPID_PUBLIC_KEY=${vapidKeys.publicKey}`);
console.log(`VAPID_PRIVATE_KEY=${vapidKeys.privateKey}`);
console.log('\nVAPID_SUBJECT=mailto:your-email@example.com');
console.log('\n' + '='.repeat(60));
console.log('\nThe public key will be used in your web app.');
console.log('The private key must be kept secret on the server.');
console.log('='.repeat(60));
