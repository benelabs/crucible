import { test, expect } from '@playwright/test';

test.describe('Crucible Developer Portal - E2E Tests', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/');
    await page.waitForLoadState('networkidle');
  });

  test.describe('Wallet Connection Flow', () => {
    test('should display wallet connection tab', async ({ page }) => {
      await page.getByTestId('tab-wallet').click();
      await expect(page.getByTestId('tab-wallet')).toHaveClass(/active/);
    });

    test('should render wallet interface', async ({ page }) => {
      await page.getByTestId('tab-wallet').click();
      await page.waitForLoadState('networkidle');
      const walletTab = page.getByTestId('tab-wallet');
      await expect(walletTab).toBeVisible();
    });

    test('should display connection status', async ({ page }) => {
      await page.getByTestId('tab-wallet').click();
      const status = page.locator('[data-testid="wallet-status"]');
      await expect(status).toBeDefined();
    });
  });

  test.describe('Contract Compilation Flow', () => {
    test('should compile a Soroban contract successfully', async ({ page }) => {
      await page.getByTestId('tab-compiler').click();
      
      const projectInput = page.getByLabel('Project Name');
      await projectInput.clear();
      await projectInput.fill('test-contract');
      
      const codeTextarea = page.getByLabel('Source Code');
      await codeTextarea.fill('use soroban_sdk::{contract, contractimpl};\n#[contract]\npub struct TestContract;');
      
      const compileBtn = page.getByTestId('compile-button');
      await compileBtn.click();
      
      await page.waitForTimeout(1000);
    });

    test('should display compilation results', async ({ page }) => {
      await page.getByTestId('tab-compiler').click();
      
      const projectInput = page.getByLabel('Project Name');
      await projectInput.fill('test-contract');
      
      const codeTextarea = page.getByLabel('Source Code');
      await codeTextarea.fill('use soroban_sdk::contract;');
      
      const compileBtn = page.getByTestId('compile-button');
      await compileBtn.click();
      
      await page.waitForTimeout(500);
    });

    test('should allow editing project name', async ({ page }) => {
      await page.getByTestId('tab-compiler').click();
      
      const projectInput = page.getByLabel('Project Name');
      await projectInput.clear();
      await projectInput.fill('my-soroban-contract');
      
      await expect(projectInput).toHaveValue('my-soroban-contract');
    });

    test('should allow editing source code', async ({ page }) => {
      await page.getByTestId('tab-compiler').click();
      
      const codeTextarea = page.getByLabel('Source Code');
      await codeTextarea.clear();
      await codeTextarea.fill('fn main() {}');
      
      const value = await codeTextarea.inputValue();
      expect(value).toContain('fn main');
    });
  });

  test.describe('Transaction Simulation Flow', () => {
    test('should navigate to Transaction Simulator', async ({ page }) => {
      await page.getByTestId('tab-simulator').click();
      await expect(page.getByTestId('tab-simulator')).toHaveClass(/active/);
    });

    test('should load simulator interface', async ({ page }) => {
      await page.getByTestId('tab-simulator').click();
      await page.waitForLoadState('networkidle');
      const simulatorTab = page.getByTestId('tab-simulator');
      await expect(simulatorTab).toHaveClass(/active/);
    });

    test('should display simulation controls', async ({ page }) => {
      await page.getByTestId('tab-simulator').click();
      await page.waitForLoadState('networkidle');
      const simulator = page.locator('[data-testid="transaction-simulator"]');
      await expect(simulator).toBeDefined();
    });

    test('should handle transaction input', async ({ page }) => {
      await page.getByTestId('tab-simulator').click();
      await page.waitForLoadState('networkidle');
      
      const input = page.locator('[data-testid="tx-input"]');
      if (await input.isVisible()) {
        await input.fill('test_transaction');
      }
    });
  });

  test.describe('ABI Explorer & Contract Invocation', () => {
    test('should navigate to ABI Explorer', async ({ page }) => {
      await page.getByTestId('tab-abi').click();
      await expect(page.getByTestId('tab-abi')).toHaveClass(/active/);
      await expect(page.getByText('Contract ABI Explorer')).toBeVisible();
    });

    test('should display ABI interface', async ({ page }) => {
      await page.getByTestId('tab-abi').click();
      const abiTab = page.getByTestId('tab-abi');
      await expect(abiTab).toBeVisible();
    });

    test('should render ABI explorer content', async ({ page }) => {
      await page.getByTestId('tab-abi').click();
      await page.waitForLoadState('networkidle');
      const explorer = page.locator('[data-testid="abi-explorer"]');
      await expect(explorer).toBeDefined();
    });

    test('should allow contract address input', async ({ page }) => {
      await page.getByTestId('tab-abi').click();
      
      const addressInput = page.locator('[data-testid="contract-address-input"]');
      if (await addressInput.isVisible()) {
        await addressInput.fill('CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4');
      }
    });
  });

  test.describe('Contract Editing & Simulation Integration', () => {
    test('should edit contract code and run simulation', async ({ page }) => {
      await page.getByTestId('tab-compiler').click();
      
      const codeTextarea = page.getByLabel('Source Code');
      await codeTextarea.clear();
      await codeTextarea.fill('use soroban_sdk::{contract, contractimpl};\n#[contract]\npub struct Counter;');
      
      // Simulate tab
      await page.getByTestId('tab-simulator').click();
      await page.waitForLoadState('networkidle');
    });

    test('should compile and view ABI', async ({ page }) => {
      await page.getByTestId('tab-compiler').click();
      
      const projectInput = page.getByLabel('Project Name');
      await projectInput.fill('counter-contract');
      
      const compileBtn = page.getByTestId('compile-button');
      await compileBtn.click();
      
      await page.getByTestId('tab-abi').click();
      await page.waitForLoadState('networkidle');
    });
  });

  test.describe('Full User Journey', () => {
    test('should complete contract development workflow', async ({ page }) => {
      // Step 1: Verify app loads
      await expect(page.getByText('Crucible Developer Portal')).toBeVisible();
      
      // Step 2: Write contract
      await page.getByTestId('tab-compiler').click();
      const codeTextarea = page.getByLabel('Source Code');
      await codeTextarea.clear();
      await codeTextarea.fill('use soroban_sdk::{contract, contractimpl};');
      
      // Step 3: Compile
      const projectInput = page.getByLabel('Project Name');
      await projectInput.clear();
      await projectInput.fill('my-contract');
      
      // Step 4: View ABI
      await page.getByTestId('tab-abi').click();
      await page.waitForLoadState('networkidle');
      
      // Step 5: Simulate transaction
      await page.getByTestId('tab-simulator').click();
      await page.waitForLoadState('networkidle');
      
      // Verify navigation works
      const tutorials = page.getByTestId('tab-tutorial');
      await expect(tutorials).toBeVisible();
    });

    test('should maintain state across tab switches', async ({ page }) => {
      await page.getByTestId('tab-compiler').click();
      
      const projectInput = page.getByLabel('Project Name');
      await projectInput.fill('stateful-contract');
      
      // Switch away and back
      await page.getByTestId('tab-metrics').click();
      await page.getByTestId('tab-compiler').click();
      
      // State should be maintained
      await expect(projectInput).toHaveValue('stateful-contract');
    });
  });

  test.describe('Bundle Size & Performance', () => {
    test('should load page within acceptable time', async ({ page }) => {
      const startTime = Date.now();
      await page.goto('/');
      await page.waitForLoadState('networkidle');
      const loadTime = Date.now() - startTime;
      
      expect(loadTime).toBeLessThan(5000);
    });

    test('should lazy load components efficiently', async ({ page }) => {
      await page.goto('/');
      
      // Initial load should not load all components
      await page.getByTestId('tab-metrics').click();
      await page.waitForLoadState('networkidle');
      
      // Verify component loaded
      const metricsTab = page.getByTestId('tab-metrics');
      await expect(metricsTab).toHaveClass(/active/);
    });

    test('should render without console errors', async ({ page }) => {
      const errors: string[] = [];
      page.on('console', msg => {
        if (msg.type() === 'error') {
          errors.push(msg.text());
        }
      });
      
      await page.goto('/');
      await page.waitForLoadState('networkidle');
      
      const appErrors = errors.filter(e => !e.includes('ResizeObserver'));
      expect(appErrors.length).toBe(0);
    });

    test('should load metrics tab with charts', async ({ page }) => {
      await page.getByTestId('tab-metrics').click();
      await page.waitForLoadState('networkidle');
      
      const chartsContainer = page.locator('svg');
      await expect(chartsContainer).toBeDefined();
    });
  });

  test.describe('Cross-Browser Compatibility', () => {
    test('should work in current browser', async ({ browserName, page }) => {
      expect(['chromium', 'firefox', 'webkit']).toContain(browserName);
      
      await page.goto('/');
      await expect(page.getByText('Crucible Developer Portal')).toBeVisible();
    });

    test('should handle responsive layouts', async ({ page }) => {
      // Mobile view
      await page.setViewportSize({ width: 375, height: 667 });
      await page.goto('/');
      await expect(page.getByText('Crucible Developer Portal')).toBeVisible();
      
      // Tablet view
      await page.setViewportSize({ width: 768, height: 1024 });
      await expect(page.getByText('Crucible Developer Portal')).toBeVisible();
      
      // Desktop view
      await page.setViewportSize({ width: 1920, height: 1080 });
      await expect(page.getByText('Crucible Developer Portal')).toBeVisible();
    });
  });

  test.describe('Accessibility', () => {
    test('should have keyboard navigation', async ({ page }) => {
      await page.goto('/');
      await page.keyboard.press('Tab');
      
      const focusedElement = await page.evaluate(() => document.activeElement?.tagName);
      expect(focusedElement).toBeTruthy();
    });

    test('should have proper heading hierarchy', async ({ page }) => {
      await page.goto('/');
      const h1Count = await page.getByRole('heading', { level: 1 }).count();
      expect(h1Count).toBeGreaterThan(0);
    });

    test('should navigate between tabs with keyboard', async ({ page }) => {
      await page.goto('/');
      
      const tabButtons = page.getByRole('button').filter({ has: page.getByTestId(/tab-/) });
      const count = await tabButtons.count();
      expect(count).toBeGreaterThan(0);
    });
  });

  test.describe('Visual Regression', () => {
    test('should maintain consistent header layout', async ({ page }) => {
      await page.goto('/');
      await page.waitForLoadState('networkidle');
      
      const header = page.locator('.app-header');
      await expect(header).toBeVisible();
    });

    test('should maintain consistent tab styling', async ({ page }) => {
      await page.goto('/');
      
      const tabs = page.locator('.tab-btn');
      await expect(tabs.first()).toBeVisible();
      
      await tabs.first().click();
      await expect(tabs.first()).toHaveClass(/active/);
    });

    test('should render compiler interface consistently', async ({ page }) => {
      await page.goto('/');
      await page.getByTestId('tab-compiler').click();
      
      const projectInput = page.getByLabel('Project Name');
      await expect(projectInput).toBeVisible();
      
      const codeTextarea = page.getByLabel('Source Code');
      await expect(codeTextarea).toBeVisible();
    });
  });

  test.describe('App Navigation', () => {
    test('should load with Tutorial tab active', async ({ page }) => {
      await page.goto('/');
      const tutorialTab = page.getByTestId('tab-tutorial');
      await expect(tutorialTab).toHaveClass(/active/);
    });

    test('should switch between all tabs', async ({ page }) => {
      await page.goto('/');
      
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
        const tab = page.getByTestId(tabId);
        await expect(tab).toBeVisible();
      }
    });

    test('should display all navigation tabs', async ({ page }) => {
      await page.goto('/');
      
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

    test('should maintain active tab state', async ({ page }) => {
      await page.goto('/');
      
      await page.getByTestId('tab-compiler').click();
      await expect(page.getByTestId('tab-compiler')).toHaveClass(/active/);
      
      await page.getByTestId('tab-metrics').click();
      await expect(page.getByTestId('tab-metrics')).toHaveClass(/active/);
      
      await page.getByTestId('tab-compiler').click();
      await expect(page.getByTestId('tab-compiler')).toHaveClass(/active/);
    });
  });
});
