import { test, expect } from '@playwright/test';

/**
 * Smoke test: Verify the app loads and basic elements are present
 */
test.describe('Smoke Tests', () => {
  test('app loads and shows title', async ({ page }) => {
    await page.goto('/');

    // Wait for WASM to load
    await page.waitForSelector('#status', { timeout: 10000 });

    // Check that we have the main navigation (be specific to avoid strict mode violations)
    await expect(page.locator('.nav-item:has-text("Identity")')).toBeVisible();
    await expect(page.locator('.nav-item:has-text("Groups")')).toBeVisible();
    await expect(page.locator('.nav-item:has-text("Wallet")')).toBeVisible();
    await expect(page.locator('.nav-item:has-text("Settings")')).toBeVisible();
  });

  test('identity is automatically generated', async ({ page }) => {
    await page.goto('/');

    // Wait for WASM to load
    await page.waitForSelector('#status', { timeout: 10000 });

    // Click Identity section
    await page.click('.nav-item:has-text("Identity")');

    // Wait for npub to appear (automatically generated on load)
    const npubDisplay = page.locator('#npub');
    await expect(npubDisplay).toBeVisible({ timeout: 5000 });

    // Verify npub format (npub1 followed by 58 characters)
    const npub = await npubDisplay.textContent();
    expect(npub).toMatch(/^npub1[a-z0-9]{58}$/);
  });
});
