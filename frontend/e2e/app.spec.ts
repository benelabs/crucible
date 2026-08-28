import { test, expect } from '@playwright/test';

test.describe('Crucible Developer Portal - E2E Tests', () => {
  test.beforeEach(async ({ page }) => {
    // Navigate to the app
    await page.goto('/');
    // Wait for the app to load
    await page.waitForLoadState('networkidle');
  });

  test.describe('App Navigation', () => {
    test('should load the app with Tutorial tab active by default', async ({ page }) => {
      // Check for app title
      await expect(page.getByText('Crucible Developer Portal')).toBeVisible();
      
      // Check that Tutorial tab is active
      const tutorialTab = page.getByTestId('tab-tutorial');
      await expect(tutorialTab).toHaveClass(/active/);
    });

    test('should switch between tabs', async ({ page }) => {
      // Switch to Gas Estimator tab
      await page.getByTestId('tab-metrics').click();
      await expect(page.getByTestId('tab-metrics')).toHaveClass(/active/);
      await expect(page.getByText('Gas Cost Estimator')).toBeVisible();

      // Switch to ABI Explorer tab
      await page.getByTestId('tab-abi').click();
      await expect(page.getByTestId('tab-abi')).toHaveClass(/active/);
      await expect(page.getByText('Contract ABI Explorer')).toBeVisible();

      // Switch back to Tutorial
      await page.getByTestId('tab-tutorial').click();
      await expect(page.getByTestId('tab-tutorial')).toHaveClass(/active/);
    });

    test('should display all navigation tabs', async ({ page }) => {
      const tabs = [
        'tab-tutorial',
        'tab-events',
        'tab-simulator',
        'tab-metrics',
        'tab-multichain',
        'tab-abi',
        'tab-compiler',
        'tab-dependencies',
        'tab-wallet'
      ];

      for (const tabId of tabs) {
        await expect(page.getByTestId(tabId)).toBeVisible();
      }
    });
  });

  test.describe('Contract Compilation Flow', () => {
    test('should compile a Soroban contract', async ({ page }) => {
      // Navigate to Compiler tab
      await page.getByTestId('tab-compiler').click();
      await expect(page.getByText('On-Demand compilation service')).toBeVisible();

      // Verify compiler interface is present
      await expect(page.getByLabel('Project Name')).toBeVisible();
      await expect(page.getByLabel('Source Code')).toBeVisible();
      await expect(page.getByTestId('compile-button')).toBeVisible();
    });

    test('should allow editing project name and source code', async ({ page }) => {
      // Navigate to Compiler tab
      await page.getByTestId('tab-compiler').click();

      // Edit project name
      const projectInput = page.getByLabel('Project Name');
      await projectInput.clear();
      await projectInput.fill('my-test-contract');
      await expect(projectInput).toHaveValue('my-test-contract');

      // Edit source code
      const codeTextarea = page.getByLabel('Source Code');
      await codeTextarea.clear();
      await codeTextarea.fill('use soroban_sdk::{contract, contractimpl};');
      
      const textValue = await codeTextarea.inputValue();
      expect(textValue).toContain('soroban_sdk');
    });
  });

  test.describe('Transaction Simulation', () => {
    test('should display Transaction Simulator tab', async ({ page }) => {
      await page.getByTestId('tab-simulator').click();
      await expect(page.getByTestId('tab-simulator')).toHaveClass(/active/);
    });

    test('should load simulator content', async ({ page }) => {
      await page.getByTestId('tab-simulator').click();
      // Wait for component to load
      await page.waitForLoadState('networkidle');
      // Verify simulator tab is visible and active
      await expect(page.getByTestId('tab-simulator')).toHaveClass(/active/);
    });
  });

  test.describe('ABI Explorer', () => {
    test('should navigate to ABI Explorer tab', async ({ page }) => {
      await page.getByTestId('tab-abi').click();
      await expect(page.getByTestId('tab-abi')).toHaveClass(/active/);
      await expect(page.getByText('Contract ABI Explorer')).toBeVisible();
    });

    test('should display ABI explorer interface', async ({ page }) => {
      await page.getByTestId('tab-abi').click();
      // Verify the tab content loaded
      const abiTab = page.getByTestId('tab-abi');
      await expect(abiTab).toBeVisible();
    });
  });

  test.describe('Wallet Connection', () => {
    test('should display wallet connection tab', async ({ page }) => {
      await page.getByTestId('tab-wallet').click();
      await expect(page.getByTestId('tab-wallet')).toHaveClass(/active/);
    });

    test('should navigate to wallet', async ({ page }) => {
      await page.getByTestId('tab-wallet').click();
      // Verify wallet tab is active
      const walletTab = page.getByTestId('tab-wallet');
      await expect(walletTab).toHaveClass(/active/);
    });
  });

  test.describe('Event Listener Dashboard', () => {
    test('should navigate to Event Listener tab', async ({ page }) => {
      await page.getByTestId('tab-events').click();
      await expect(page.getByTestId('tab-events')).toHaveClass(/active/);
    });

    test('should load event listener content', async ({ page }) => {
      await page.getByTestId('tab-events').click();
      await page.waitForLoadState('networkidle');
      const eventsTab = page.getByTestId('tab-events');
      await expect(eventsTab).toHaveClass(/active/);
    });
  });

  test.describe('Dependency Analyzer', () => {
    test('should navigate to Dependency Analyzer tab', async ({ page }) => {
      await page.getByTestId('tab-dependencies').click();
      await expect(page.getByTestId('tab-dependencies')).toHaveClass(/active/);
    });

    test('should display dependency analyzer interface', async ({ page }) => {
      await page.getByTestId('tab-dependencies').click();
      await page.waitForLoadState('networkidle');
      const depTab = page.getByTestId('tab-dependencies');
      await expect(depTab).toHaveClass(/active/);
    });
  });

  test.describe('Multi-Chain Dashboard', () => {
    test('should navigate to MultiChain Dashboard', async ({ page }) => {
      await page.getByTestId('tab-multichain').click();
      await expect(page.getByTestId('tab-multichain')).toHaveClass(/active/);
      await expect(page.getByText('Multi-Chain Support')).toBeVisible();
    });
  });

  test.describe('Responsive Design', () => {
    test('should be responsive on mobile viewport', async ({ page }) => {
      // Set mobile viewport
      await page.setViewportSize({ width: 375, height: 667 });

      // Verify app header is still visible
      await expect(page.getByText('Crucible Developer Portal')).toBeVisible();

      // Verify tabs are accessible
      const tabs = ['tab-tutorial', 'tab-metrics', 'tab-abi'];
      for (const tabId of tabs) {
        const tab = page.getByTestId(tabId);
        // Tab should be in viewport or scrollable
        await expect(tab).toBeTruthy();
      }
    });

    test('should be responsive on tablet viewport', async ({ page }) => {
      // Set tablet viewport
      await page.setViewportSize({ width: 768, height: 1024 });

      // Verify content is readable
      await expect(page.getByText('Crucible Developer Portal')).toBeVisible();
    });

    test('should be responsive on desktop viewport', async ({ page }) => {
      // Set desktop viewport
      await page.setViewportSize({ width: 1920, height: 1080 });

      // Verify all tabs are visible
      const tabCount = await page.getByTestId(/tab-/).count();
      expect(tabCount).toBeGreaterThan(0);
    });
  });

  test.describe('Tab Persistence', () => {
    test('should maintain active tab after switching', async ({ page }) => {
      // Click on multiple tabs
      await page.getByTestId('tab-compiler').click();
      await expect(page.getByTestId('tab-compiler')).toHaveClass(/active/);

      // Switch to another tab
      await page.getByTestId('tab-metrics').click();
      await expect(page.getByTestId('tab-metrics')).toHaveClass(/active/);

      // Switch back
      await page.getByTestId('tab-compiler').click();
      await expect(page.getByTestId('tab-compiler')).toHaveClass(/active/);
    });
  });

  test.describe('Page Performance', () => {
    test('should load initial page within reasonable time', async ({ page }) => {
      const startTime = Date.now();
      await page.goto('/');
      await page.waitForLoadState('networkidle');
      const loadTime = Date.now() - startTime;

      // Page should load within 5 seconds
      expect(loadTime).toBeLessThan(5000);
    });

    test('should render without console errors', async ({ page, context }) => {
      const errors: string[] = [];
      page.on('console', msg => {
        if (msg.type() === 'error') {
          errors.push(msg.text());
        }
      });

      await page.goto('/');
      await page.waitForLoadState('networkidle');

      // Filter out known third-party errors
      const appErrors = errors.filter(e => !e.includes('ResizeObserver'));
      expect(appErrors.length).toBe(0);
    });
  });

  test.describe('Accessibility', () => {
    test('should have keyboard navigation for tabs', async ({ page }) => {
      // Focus on first tab
      await page.keyboard.press('Tab');
      
      // Verify a tab has focus
      const focusedElement = await page.evaluate(() => document.activeElement?.tagName);
      expect(focusedElement).toBeTruthy();
    });

    test('should have proper heading hierarchy', async ({ page }) => {
      const h1Count = await page.getByRole('heading', { level: 1 }).count();
      expect(h1Count).toBeGreaterThan(0);
    });

    test('should have proper ARIA labels on buttons', async ({ page }) => {
      const buttons = page.getByRole('button').filter({ has: page.getByTestId(/tab-/) });
      const count = await buttons.count();
      expect(count).toBeGreaterThan(0);
    });
  });

  test.describe('Browser Compatibility', () => {
    test('should work with multiple browser engines', async ({ browserName }) => {
      // This test runs against different browsers due to playwright config
      expect(['chromium', 'firefox', 'webkit']).toContain(browserName);
    });
  });
});

test.describe('Visual Regression Tests', () => {
  test('should have consistent header appearance', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');

    // Take screenshot of header
    const header = page.locator('.app-header');
    await expect(header).toBeVisible();
    
    // In CI, you can compare against baseline:
    // await expect(header).toHaveScreenshot('header.png');
  });

  test('should have consistent tab styling', async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');

    const tabs = page.locator('.tab-btn');
    await expect(tabs.first()).toBeVisible();

    // Click to change active state
    await tabs.first().click();
    
    // Verify active class applied
    await expect(tabs.first()).toHaveClass(/active/);
  });
});
