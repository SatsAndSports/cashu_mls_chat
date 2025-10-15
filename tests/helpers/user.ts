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

    // Wait for modal to show successful publication
    try {
      await this.page.waitForSelector('text=Published to 1/1 relays', { timeout: 10000 });
      console.log(`[${this.name}] KeyPackage published successfully`);

      // Wait for Close button container to be visible
      await this.page.waitForSelector('#kp-close-button-container', { state: 'visible', timeout: 5000 });

      // Close the modal by clicking the specific button inside the KeyPackage modal
      await this.page.click('#kp-close-button-container button');
      console.log(`[${this.name}] Clicked Close button`);

      // Wait for modal to actually disappear
      await this.page.waitForSelector('#creating-keypackage-modal', { state: 'hidden', timeout: 5000 });
      console.log(`[${this.name}] Modal closed`);
    } catch (err) {
      console.error(`[${this.name}] KeyPackage creation may have failed`);
      await this.screenshot('keypackage-error');
      throw err;
    }

    console.log(`[${this.name}] KeyPackage created and published`);
  }

  /**
   * Create a new group and invite a member
   */
  async createGroup(groupName: string, inviteNpub: string) {
    console.log(`[${this.name}] Creating group: ${groupName}`);

    // Navigate to Groups
    await this.page.click('.nav-item:has-text("Groups")');

    // Click Create New Group
    await this.page.click('button:has-text("Create New Group")');

    // Fill in group details
    await this.page.fill('#create-group-name', groupName);
    await this.page.fill('#create-group-description', 'Test group description');

    // Fill in first member npub
    await this.page.fill('#create-group-first-member', inviteNpub);

    // Click Create Group button (creates the group and sends invite)
    await this.page.click('button:has-text("Create Group")');

    // Wait for success message in the invite-details-modal
    await this.page.waitForSelector('text=Group Created Successfully', { timeout: 15000 });
    console.log(`[${this.name}] Group creation succeeded`);

    // Wait for Close button to appear in invite-details-modal
    await this.page.waitForSelector('#invite-close-button-container', { state: 'visible', timeout: 5000 });

    // Close the invite-details-modal
    await this.page.click('#invite-close-button-container button');
    await this.page.waitForSelector('#invite-details-modal', { state: 'hidden', timeout: 5000 });
    console.log(`[${this.name}] Modal closed`);

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

    // Wait for groups list to load
    await this.page.waitForSelector('#groups-list', { timeout: 5000 });

    // Wait for the specific group to be visible
    try {
      await this.page.waitForSelector(`.group-item:has-text("${groupName}")`, { timeout: 5000 });
    } catch (err) {
      // Debug: screenshot and list what groups are visible
      const groupTexts = await this.page.locator('.group-item').allTextContents();
      console.error(`[${this.name}] Group "${groupName}" not found. Available groups:`, groupTexts);
      await this.screenshot('group-not-found');
      throw err;
    }

    // Click on the specific group item
    console.log(`[${this.name}] Clicking group item...`);
    await this.page.click(`.group-item:has-text("${groupName}")`);

    // Check the current URL/hash
    const url = this.page.url();
    console.log(`[${this.name}] URL after click: ${url}`);

    // Wait for chat to open
    try {
      await this.page.waitForSelector('#chat-input', { timeout: 5000 });
      console.log(`[${this.name}] Chat opened: ${groupName}`);
    } catch (err) {
      // Debug: check what section is visible
      const sections = ['#identity-section', '#groups-section', '#wallet-section', '#chat-section'];
      for (const section of sections) {
        const isVisible = await this.page.locator(section).isVisible();
        if (isVisible) {
          console.log(`[${this.name}] Currently visible section: ${section}`);
        }
      }
      await this.screenshot('chat-not-opening');
      throw err;
    }
  }

  /**
   * Send a message in the current chat
   */
  async sendMessage(text: string) {
    console.log(`[${this.name}] Sending message: "${text}"`);

    // Fill message input
    await this.page.fill('#chat-input', text);

    // Click send button
    await this.page.click('#send-button');

    console.log(`[${this.name}] Message sent (clicked send button)`);
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
