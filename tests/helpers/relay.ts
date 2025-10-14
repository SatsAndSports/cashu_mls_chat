import { spawn, ChildProcess } from 'child_process';
import { promisify } from 'util';
import { exec } from 'child_process';
import * as fs from 'fs/promises';
import * as path from 'path';
import * as os from 'os';

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

  // Create minimal config file for relay
  const configDir = await fs.mkdtemp(path.join(os.tmpdir(), 'nostr-relay-'));
  const configPath = path.join(configDir, 'config.toml');
  const dbPath = path.join(configDir, 'nostr.db');

  const config = `
[info]
relay_url = "ws://localhost:${port}"
name = "Test Relay"
description = "Nostr relay for testing"

[network]
port = ${port}
address = "127.0.0.1"

[database]
data_directory = "${configDir}"

[limits]
max_event_bytes = 2097152
`;

  await fs.writeFile(configPath, config);

  // Start the relay process
  const relayProcess: ChildProcess = spawn(
    'nostr-rs-relay',
    ['--config', configPath],
    {
      stdio: ['ignore', 'pipe', 'pipe']
    }
  );

  // Log relay output for debugging
  relayProcess.stdout?.on('data', (data) => {
    console.log(`[relay] ${data.toString().trim()}`);
  });

  relayProcess.stderr?.on('data', (data) => {
    console.error(`[relay] ${data.toString().trim()}`);
  });

  relayProcess.on('error', (error) => {
    console.error(`❌ Relay process error: ${error.message}`);
  });

  // Wait for relay to start (look for "listening" in output)
  await new Promise((resolve) => {
    const timeout = setTimeout(resolve, 5000); // Fallback timeout

    const checkOutput = (data: Buffer) => {
      const output = data.toString();
      if (output.includes('listening') || output.includes('started')) {
        clearTimeout(timeout);
        resolve(null);
      }
    };

    relayProcess.stdout?.on('data', checkOutput);
    relayProcess.stderr?.on('data', checkOutput);
  });

  console.log(`✅ Relay started on ws://localhost:${port}`);

  // Return cleanup function
  return async () => {
    console.log(`🛑 Stopping relay (port ${port})...`);

    if (relayProcess.pid) {
      relayProcess.kill('SIGTERM');

      // Wait for process to exit
      await new Promise<void>((resolve) => {
        relayProcess.on('exit', () => resolve());
        // Fallback timeout
        setTimeout(() => {
          if (!relayProcess.killed) {
            relayProcess.kill('SIGKILL');
          }
          resolve();
        }, 2000);
      });
    }

    // Clean up temp directory
    try {
      await fs.rm(configDir, { recursive: true, force: true });
    } catch (err) {
      // Ignore cleanup errors
    }

    console.log(`✅ Relay stopped and cleaned up`);
  };
}
