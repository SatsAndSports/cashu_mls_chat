import { ensureRelayRunning } from './helpers/relay';

/**
 * Global setup for Playwright tests
 * Runs once before all tests
 */
export default async function globalSetup() {
  console.log('\n🔧 Global Setup: Ensuring Nostr relay is running...\n');

  // Check if relay is running, start if needed
  // Relay will stay running even after tests finish
  await ensureRelayRunning(8080);

  console.log('✅ Global Setup Complete\n');
}
