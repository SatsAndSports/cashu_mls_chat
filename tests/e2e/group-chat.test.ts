import { test, expect } from '@playwright/test';
import { TestUser } from '../helpers/user';

/**
 * Group Chat E2E Tests
 *
 * Tests the core functionality: encrypted group messaging between two users
 *
 * Note: Relay is managed globally (see tests/global-setup.ts)
 * It runs on ws://localhost:8080 and stays running across test runs
 */
test.describe('Group Chat', () => {
  test('two users can exchange messages', async ({ browser }) => {
    // Create two isolated browser contexts (like two different users)
    const aliceContext = await browser.newContext();
    const bobContext = await browser.newContext();

    // Set OVERRIDE_DEFAULT_RELAYS for both contexts
    // This ensures KeyPackages and messages use the test relay
    await aliceContext.addInitScript(() => {
      (window as any).OVERRIDE_DEFAULT_RELAYS = ['ws://localhost:8080'];
    });
    await bobContext.addInitScript(() => {
      (window as any).OVERRIDE_DEFAULT_RELAYS = ['ws://localhost:8080'];
    });

    const alicePage = await aliceContext.newPage();
    const bobPage = await bobContext.newPage();

    // Create user helpers
    const alice = new TestUser(alicePage, 'Alice');
    const bob = new TestUser(bobPage, 'Bob');

    try {
      console.log('\n=== Test Setup ===');

      // Initialize both users
      await alice.init();
      await bob.init();

      // Get Bob's npub (Alice will need it to invite him)
      const bobNpub = await bob.getNpub();

      console.log('\n=== KeyPackage Creation ===');

      // Bob creates a KeyPackage (needed for Alice to invite him)
      await bob.createKeyPackage();

      console.log('\n=== Group Creation ===');

      // Alice creates a group and invites Bob
      await alice.createGroup('Test Group', bobNpub);

      console.log('\n=== Waiting for Welcome ===');

      // Bob should automatically receive the Welcome message and join
      await bob.waitForGroup('Test Group', 20000);
      console.log('✅ Bob received Welcome and group appeared');

      console.log('\n=== Opening Chats ===');

      // Both users open the chat
      console.log('Alice opening chat...');
      await alice.openChat('Test Group');
      console.log('✅ Alice chat opened');

      console.log('Bob opening chat...');
      await bob.openChat('Test Group');
      console.log('✅ Bob chat opened');

      console.log('\n=== Message Exchange ===');

      // Alice sends first message
      await alice.sendMessage('Hello Bob!');
      await bob.waitForMessage('Hello Bob!', 15000);

      // Bob replies
      await bob.sendMessage('Hi Alice!');
      await alice.waitForMessage('Hi Alice!', 15000);

      // Send a few more messages to ensure it's stable
      await alice.sendMessage('How are you?');
      await bob.waitForMessage('How are you?', 15000);

      await bob.sendMessage('Great! Testing works!');
      await alice.waitForMessage('Great! Testing works!', 15000);

      console.log('\n=== Test Complete ===');
      console.log('✅ Successfully exchanged 4 messages!');

    } catch (error) {
      // Take screenshots on failure
      await alice.screenshot('failure-alice');
      await bob.screenshot('failure-bob');
      throw error;
    } finally {
      // Clean up browser contexts
      await aliceContext.close();
      await bobContext.close();
    }
  });
});
