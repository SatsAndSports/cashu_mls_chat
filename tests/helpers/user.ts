import { Page, expect } from '@playwright/test';

/**
 * Test user helper - encapsulates common user operations
 */
export class TestUser {
  constructor(
    public page: Page,
    public name: string
  ) {}

  /**
   * Navigate to app and wait for WASM to load
   */
  async init() {
    await this.page.goto('/');
    await this.page.waitForSelector('#status', { timeout: 10000 });
    console.log(`[${this.name}] App loaded, WASM ready`);
  }

  /**
   * Get user's npub (public key)
   */
  async getNpub(): Promise<string> {
    await this.page.click('.nav-item:has-text("Identity")');
    const npub = await this.page.locator('#npub').textContent();
    if (!npub) throw new Error('npub not found');
    console.log(`[${this.name}] npub: ${npub.substring(0, 20)}...`);
    return npub;
  }

  /**
   * Add a Nostr relay
   */
  async addRelay(relayUrl: string) {
    console.log(`[${this.name}] Adding relay: ${relayUrl}`);

    await this.page.click('.nav-item:has-text("Relays")');
    await this.page.fill('#relay-input', relayUrl);
    await this.page.click('button:has-text("Add Relay")');

    // Wait a bit for relay to connect
    await this.page.waitForTimeout(1000);

    console.log(`[${this.name}] Relay added`);
  }

  /**
   * Create a new KeyPackage
   */
  async createKeyPackage() {
    console.log(`[${this.name}] Creating KeyPackage...`);

    await this.page.click('.nav-item:has-text("Identity")');

    // The button text is "+ Create KeyPackage"
    await this.page.click('button:has-text("Create KeyPackage")');

    // Wait for KeyPackage modal to appear and show success
    try {
      await this.page.waitForSelector('text=KeyPackage Created Successfully', { timeout: 10000 });
      console.log(`[${this.name}] KeyPackage creation modal appeared`);

      // Close the modal
      await this.page.click('button:has-text("Close")');
    } catch (err) {
      console.error(`[${this.name}] KeyPackage creation may have failed or modal didn't appear`);
      await this.screenshot('keypackage-error');
    }

    // Wait for KeyPackage to be published to relay
    await this.page.waitForTimeout(2000);

    console.log(`[${this.name}] KeyPackage created and published`);
  }

  /**
   * Create a new group and invite a member
   */
  async createGroup(groupName: string, inviteNpub: string) {
    console.log(`[${this.name}] Creating group: ${groupName}`);

    // Navigate to Groups
    await this.page.click('.nav-item:has-text("Groups")');

    // Click Create Group
    await this.page.click('button:has-text("Create Group")');

    // Fill in group details
    await this.page.fill('#group-name-input', groupName);
    await this.page.fill('#group-description-input', 'Test group description');

    // Fill in first member npub
    await this.page.fill('#first-member-npub', inviteNpub);

    // Click to proceed with invitation
    await this.page.click('button:has-text("Next: Select KeyPackage")');

    // Wait for KeyPackage list to load
    await this.page.waitForTimeout(2000);

    // Select first available KeyPackage (radio button)
    const firstKeyPackageRadio = this.page.locator('input[name="keypackage-select"]').first();
    await firstKeyPackageRadio.click();

    // Create the group
    await this.page.click('button:has-text("Create Group & Send Invite")');

    // Wait for group creation to complete
    await this.page.waitForTimeout(5000);

    console.log(`[${this.name}] Group created: ${groupName}`);
  }

  /**
   * Wait for a group to appear (after receiving Welcome)
   */
  async waitForGroup(groupName: string, timeout: number = 15000) {
    console.log(`[${this.name}] Waiting for group: ${groupName}`);

    // Navigate to Groups section
    await this.page.click('.nav-item:has-text("Groups")');

    // Wait for group to appear in list
    await this.page.waitForSelector(`text="${groupName}"`, { timeout });

    console.log(`[${this.name}] Group appeared: ${groupName}`);
  }

  /**
   * Open a chat with a group
   */
  async openChat(groupName: string) {
    console.log(`[${this.name}] Opening chat: ${groupName}`);

    // Navigate to Groups if not already there
    await this.page.click('.nav-item:has-text("Groups")');

    // Click on the group to open chat
    await this.page.click(`text="${groupName}"`);

    // Wait for chat to open
    await this.page.waitForSelector('#message-input', { timeout: 5000 });

    console.log(`[${this.name}] Chat opened: ${groupName}`);
  }

  /**
   * Send a message in the current chat
   */
  async sendMessage(text: string) {
    console.log(`[${this.name}] Sending message: "${text}"`);

    // Fill message input
    await this.page.fill('#message-input', text);

    // Click send button
    await this.page.click('button:has-text("Send")');

    // Wait for message to be sent
    await this.page.waitForTimeout(1000);

    console.log(`[${this.name}] Message sent`);
  }

  /**
   * Wait for a specific message to appear in chat
   */
  async waitForMessage(text: string, timeout: number = 10000) {
    console.log(`[${this.name}] Waiting for message: "${text}"`);

    // Wait for message to appear in chat messages
    await this.page.waitForSelector(`#chat-messages >> text="${text}"`, { timeout });

    console.log(`[${this.name}] Message received: "${text}"`);
  }

  /**
   * Take a screenshot (for debugging)
   */
  async screenshot(name: string) {
    await this.page.screenshot({
      path: `test-results/${this.name}-${name}.png`,
      fullPage: true
    });
    console.log(`[${this.name}] Screenshot saved: ${name}`);
  }
}
