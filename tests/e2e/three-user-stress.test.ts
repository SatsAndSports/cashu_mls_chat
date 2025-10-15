import { test, expect } from '@playwright/test';
import { TestUser } from '../helpers/user';

/**
 * Three-User Group Chat Stress Test
 *
 * Stress tests the system with 50 messages from each user (150 total messages)
 * 1. Alice creates group and invites Bob
 * 2. Bob invites Carol to the group
 * 3. All three send 50 messages each in round-robin fashion
 */
test.describe('Three-User Group Chat Stress Test', () => {
  test('three users can exchange 150 messages (50 each)', async ({ browser }) => {
    // Increase timeout for this stress test
    test.setTimeout(180000); // 3 minutes

    // Create three isolated browser contexts
    const aliceContext = await browser.newContext();
    const bobContext = await browser.newContext();
    const carolContext = await browser.newContext();

    // Set OVERRIDE_DEFAULT_RELAYS for all contexts
    await aliceContext.addInitScript(() => {
      (window as any).OVERRIDE_DEFAULT_RELAYS = ['ws://localhost:8080'];
    });
    await bobContext.addInitScript(() => {
      (window as any).OVERRIDE_DEFAULT_RELAYS = ['ws://localhost:8080'];
    });
    await carolContext.addInitScript(() => {
      (window as any).OVERRIDE_DEFAULT_RELAYS = ['ws://localhost:8080'];
    });

    const alicePage = await aliceContext.newPage();
    const bobPage = await bobContext.newPage();
    const carolPage = await carolContext.newPage();

    // Create user helpers
    const alice = new TestUser(alicePage, 'Alice');
    const bob = new TestUser(bobPage, 'Bob');
    const carol = new TestUser(carolPage, 'Carol');

    try {
      console.log('\n=== Test Setup ===');

      // Initialize all users
      await alice.init();
      await bob.init();
      await carol.init();

      // Get npubs
      const bobNpub = await bob.getNpub();
      const carolNpub = await carol.getNpub();

      console.log('\n=== KeyPackage Creation ===');

      // Bob and Carol need KeyPackages to be invited
      await bob.createKeyPackage();
      await carol.createKeyPackage();

      console.log('\n=== Group Creation (Alice invites Bob) ===');

      // Alice creates group and invites Bob (Bob needs to be admin to invite Carol)
      await alice.createGroup('Stress Test Group', bobNpub, true);

      console.log('\n=== Waiting for Bob to join ===');

      // Bob receives Welcome and joins
      await bob.waitForGroup('Stress Test Group', 20000);
      console.log('✅ Bob received Welcome and group appeared');

      console.log('\n=== Bob invites Carol ===');

      // Bob invites Carol to the group
      await bob.inviteMember('Stress Test Group', carolNpub);

      console.log('\n=== Waiting for Carol to join ===');

      // Carol receives Welcome and joins
      await carol.waitForGroup('Stress Test Group', 20000);
      console.log('✅ Carol received Welcome and group appeared');

      console.log('\n=== Opening Chats ===');

      // All three users open the chat
      await alice.openChat('Stress Test Group');
      await bob.openChat('Stress Test Group');
      await carol.openChat('Stress Test Group');
      console.log('✅ All chats opened');

      console.log('\n=== Stress Test: Sending 150 messages (50 per user, all concurrent) ===');

      const startTime = Date.now();

      // Send 50 messages from each user concurrently (all at once)
      await Promise.all([
        // Alice sends 50 messages as fast as possible
        (async () => {
          for (let i = 1; i <= 50; i++) {
            await alice.sendMessage(`Alice message ${i}`);
          }
          console.log('✅ Alice finished sending 50 messages');
        })(),

        // Bob sends 50 messages as fast as possible
        (async () => {
          for (let i = 1; i <= 50; i++) {
            await bob.sendMessage(`Bob message ${i}`);
          }
          console.log('✅ Bob finished sending 50 messages');
        })(),

        // Carol sends 50 messages as fast as possible
        (async () => {
          for (let i = 1; i <= 50; i++) {
            await carol.sendMessage(`Carol message ${i}`);
          }
          console.log('✅ Carol finished sending 50 messages');
        })(),
      ]);

      console.log('✅ All 150 messages sent');

      console.log('\n=== Verifying Message Delivery ===');

      // Wait for and verify key messages (first, middle, last from each user)
      const verifyMessages = [
        'Alice message 1',
        'Alice message 25',
        'Alice message 50',
        'Bob message 1',
        'Bob message 25',
        'Bob message 50',
        'Carol message 1',
        'Carol message 25',
        'Carol message 50',
      ];

      // Give extra time for all messages to propagate
      const verifyTimeout = 30000;

      for (const msg of verifyMessages) {
        await alice.waitForMessage(msg, verifyTimeout);
        await bob.waitForMessage(msg, verifyTimeout);
        await carol.waitForMessage(msg, verifyTimeout);
      }

      const endTime = Date.now();
      const duration = ((endTime - startTime) / 1000).toFixed(1);

      console.log('\n=== Test Complete ===');
      console.log(`✅ Successfully exchanged 150 messages between 3 users!`);
      console.log(`⏱️  Duration: ${duration} seconds`);
      console.log(`📊 Throughput: ${(150 / parseFloat(duration)).toFixed(1)} messages/second`);

    } catch (error) {
      // Take screenshots on failure
      await alice.screenshot('stress-failure-alice');
      await bob.screenshot('stress-failure-bob');
      await carol.screenshot('stress-failure-carol');
      throw error;
    } finally {
      // Clean up browser contexts
      await aliceContext.close();
      await bobContext.close();
      await carolContext.close();
    }
  });
});
