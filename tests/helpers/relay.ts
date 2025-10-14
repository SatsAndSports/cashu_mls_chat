import { exec, ChildProcess } from 'child_process';
import { promisify } from 'util';

const execAsync = promisify(exec);

/**
 * Start a local Nostr relay for testing
 *
 * Prerequisites:
 * - nostr-rs-relay installed: cargo install nostr-rs-relay
 *
 * @param port Port to run the relay on (default: 8080)
 * @returns Cleanup function to stop the relay
 */
export async function startRelay(port: number = 8080): Promise<() => Promise<void>> {
  console.log(`🚀 Starting Nostr relay on port ${port}...`);

  // Check if nostr-rs-relay is installed
  try {
    await execAsync('which nostr-rs-relay');
  } catch (err) {
    throw new Error(
      'nostr-rs-relay not found. Install it with: cargo install nostr-rs-relay'
    );
  }

  // Start the relay process
  const relayProcess: ChildProcess = exec(
    `nostr-rs-relay --port ${port}`,
    (error, stdout, stderr) => {
      if (error && !error.killed) {
        console.error(`❌ Relay error: ${error.message}`);
      }
    }
  );

  // Wait for relay to start (generous timeout)
  await new Promise(resolve => setTimeout(resolve, 3000));

  console.log(`✅ Relay started on ws://localhost:${port}`);

  // Return cleanup function
  return async () => {
    if (relayProcess.pid) {
      relayProcess.kill();
      console.log(`🛑 Relay stopped (port ${port})`);
    }
  };
}

/**
 * Wait for relay to be ready by attempting WebSocket connection
 */
export async function waitForRelay(url: string, maxAttempts: number = 10): Promise<void> {
  for (let i = 0; i < maxAttempts; i++) {
    try {
      // Try to connect (will implement when we add WebSocket tests)
      await new Promise(resolve => setTimeout(resolve, 500));
      console.log(`✅ Relay ready at ${url}`);
      return;
    } catch (err) {
      if (i === maxAttempts - 1) {
        throw new Error(`Relay not ready after ${maxAttempts} attempts`);
      }
      await new Promise(resolve => setTimeout(resolve, 1000));
    }
  }
}
