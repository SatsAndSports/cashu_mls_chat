import { test, expect } from '@playwright/test';
import { TestUser } from '../helpers/user';

/**
 * Three-User Group Chat Test
 *
 * Tests group messaging with 3 users:
 * 1. Alice creates group and invites Bob
 * 2. Bob invites Carol to the group
 * 3. All three exchange messages
 */
test.describe('Three-User Group Chat', () => {
  test('three users can exchange messages', async ({ browser }) => {
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
      await alice.createGroup('Test Group', bobNpub, true);

      console.log('\n=== Waiting for Bob to join ===');

      // Bob receives Welcome and joins
      await bob.waitForGroup('Test Group', 20000);
      console.log('✅ Bob received Welcome and group appeared');

      console.log('\n=== Bob invites Carol ===');

      // Bob invites Carol to the group
      await bob.inviteMember('Test Group', carolNpub);

      console.log('\n=== Waiting for Carol to join ===');

      // Carol receives Welcome and joins
      await carol.waitForGroup('Test Group', 20000);
      console.log('✅ Carol received Welcome and group appeared');

      console.log('\n=== Opening Chats ===');

      // All three users open the chat
      console.log('Alice opening chat...');
      await alice.openChat('Test Group');
      console.log('✅ Alice chat opened');

      console.log('Bob opening chat...');
      await bob.openChat('Test Group');
      console.log('✅ Bob chat opened');

      console.log('Carol opening chat...');
      await carol.openChat('Test Group');
      console.log('✅ Carol chat opened');

      console.log('\n=== Message Exchange ===');

      // Alice sends to all
      await alice.sendMessage('Hello everyone!');
      await bob.waitForMessage('Hello everyone!', 15000);
      await carol.waitForMessage('Hello everyone!', 15000);

      // Bob replies
      await bob.sendMessage('Hi Alice and Carol!');
      await alice.waitForMessage('Hi Alice and Carol!', 15000);
      await carol.waitForMessage('Hi Alice and Carol!', 15000);

      // Carol joins the conversation
      await carol.sendMessage('Hello Alice and Bob!');
      await alice.waitForMessage('Hello Alice and Bob!', 15000);
      await bob.waitForMessage('Hello Alice and Bob!', 15000);

      // More back and forth
      await alice.sendMessage('Great to have everyone here!');
      await bob.waitForMessage('Great to have everyone here!', 15000);
      await carol.waitForMessage('Great to have everyone here!', 15000);

      await carol.sendMessage('Testing works perfectly!');
      await alice.waitForMessage('Testing works perfectly!', 15000);
      await bob.waitForMessage('Testing works perfectly!', 15000);

      console.log('\n=== Test Complete ===');
      console.log('✅ Successfully exchanged 5 messages between 3 users!');

    } catch (error) {
      // Take screenshots on failure
      await alice.screenshot('failure-alice');
      await bob.screenshot('failure-bob');
      await carol.screenshot('failure-carol');
      throw error;
    } finally {
      // Clean up browser contexts
      await aliceContext.close();
      await bobContext.close();
      await carolContext.close();
    }
  });
});
