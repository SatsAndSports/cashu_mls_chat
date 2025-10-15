import { test, expect } from '@playwright/test';

/**
 * Relay Management Tests
 *
 * Tests basic relay configuration functionality
 */
test.describe('Relay Management', () => {
  test('can add relay', async ({ browser }) => {
    // Start with empty relay list
    const context = await browser.newContext();
    await context.addInitScript(() => {
      (window as any).OVERRIDE_DEFAULT_RELAYS = [];
    });

    const page = await context.newPage();

    // Navigate to app
    await page.goto('/');
    await page.waitForSelector('#status', { timeout: 10000 });

    // Navigate to Relays section
    await page.click('.nav-item:has-text("Relays")');

    // Wait for relay list to actually render
    // Either it shows "No relays configured" or has relay items
    await page.waitForFunction(() => {
      const list = document.getElementById('relays-list');
      return list && list.innerHTML.length > 0;
    }, { timeout: 5000 });

    // Check initial relay count (should be 0)
    const initialRelays = await page.locator('#relays-list > div:has(code)').count();

    // Log what we got for debugging
    if (initialRelays !== 0) {
      const relayTexts = await page.locator('#relays-list code').allTextContents();
      console.log(`❌ Expected 0 relays but got ${initialRelays}:`, relayTexts);
    }

    expect(initialRelays).toBe(0);
    console.log(`Initial relay count: ${initialRelays}`);

    // Add the test relay (localhost:8080 which is actually running)
    const testRelay = 'ws://localhost:8080';
    await page.fill('#relay-input', testRelay);
    await page.click('button:has-text("Add Relay")');

    // Wait for relay to be added
    await page.waitForSelector(`#relays-list code:has-text("${testRelay}")`, { timeout: 5000 });

    // Verify relay was added (should be 1 now)
    const newRelayCount = await page.locator('#relays-list > div:has(code)').count();
    expect(newRelayCount).toBe(1);
    await expect(page.locator(`#relays-list code:has-text("${testRelay}")`)).toBeVisible();
    console.log(`✅ Relay added: ${testRelay}`);

    // TODO: Test removal (confirm() dialog is tricky in Playwright)

    await context.close();
  });

  test('OVERRIDE_DEFAULT_RELAYS works correctly', async ({ browser }) => {
    // Create context with OVERRIDE_DEFAULT_RELAYS
    const context = await browser.newContext();
    await context.addInitScript(() => {
      (window as any).OVERRIDE_DEFAULT_RELAYS = ['ws://localhost:8080'];
    });

    const page = await context.newPage();

    // Navigate to app
    await page.goto('/');
    await page.waitForSelector('#status', { timeout: 10000 });

    // Navigate to Relays section
    await page.click('.nav-item:has-text("Relays")');

    // Should only have 1 relay (localhost:8080)
    const relayCount = await page.locator('#relays-list > div:has(code)').count();
    expect(relayCount).toBe(1);

    // Verify it's the test relay
    await expect(page.locator('#relays-list code:has-text("ws://localhost:8080")')).toBeVisible();
    console.log('✅ OVERRIDE_DEFAULT_RELAYS working correctly');

    await context.close();
  });
});
