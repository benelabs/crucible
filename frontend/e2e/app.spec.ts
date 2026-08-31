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
      await expect(page.getByText('Wallet Connector')).toBeVisible();
    });

    test('should list available wallets when disconnected', async ({ page }) => {
      await page.getByTestId('tab-wallet').click();
      await expect(page.getByTestId('wallet-list')).toBeVisible();
      await expect(page.getByTestId('connect-freighter')).toBeVisible();
    });

    test('should connect and disconnect a wallet', async ({ page }) => {
      await page.getByTestId('tab-wallet').click();

      await page.getByTestId('connect-freighter').click();
      await expect(page.getByTestId('connecting-panel')).toBeVisible();

      await expect(page.getByTestId('connected-panel')).toBeVisible({ timeout: 5000 });
      await expect(page.getByTestId('connected-pubkey')).toBeVisible();

      await page.getByTestId('disconnect-button').click();
      await expect(page.getByTestId('wallet-list')).toBeVisible();
    });

    test('should switch target network', async ({ page }) => {
      await page.getByTestId('tab-wallet').click();

      await page.getByTestId('network-tab-mainnet').click();
      await expect(page.getByTestId('network-tab-mainnet')).toHaveClass(/active/);
    });
  });

  test.describe('Contract Compilation Flow', () => {
    test('should allow editing project name and source code', async ({ page }) => {
      await page.getByTestId('tab-compiler').click();

      const projectInput = page.getByLabel('Project Name');
      await projectInput.clear();
      await projectInput.fill('my-soroban-contract');
      await expect(projectInput).toHaveValue('my-soroban-contract');

      const codeTextarea = page.getByLabel('Source Code');
      await codeTextarea.fill('use soroban_sdk::{contract, contractimpl};\n#[contract]\npub struct TestContract;');
      await expect(codeTextarea).toHaveValue(/TestContract/);
    });

    test('should trigger compilation and show build output panel', async ({ page }) => {
      await page.getByTestId('tab-compiler').click();

      const compileBtn = page.getByTestId('compile-button');
      await compileBtn.click();

      await expect(page.getByTestId('compiler-result')).toBeVisible();
    });
  });

  test.describe('Interactive Challenge Engine', () => {
    test('should load and display challenge content', async ({ page }) => {
      await page.getByTestId('tab-challenges').click();

      const challengeEngine = page.getByTestId('challenge-engine');
      await expect(challengeEngine).toBeVisible();
      await expect(page.locator('h2')).toContainText('Soroban Counter Contract');
    });

    test('should run tests and display results', async ({ page }) => {
      await page.getByTestId('tab-challenges').click();

      await page.getByTestId('run-tests-btn').click();

      await expect(page.getByTestId('test-results')).toBeVisible({ timeout: 3000 });
    });

    test('should reveal and collapse hints', async ({ page }) => {
      await page.getByTestId('tab-challenges').click();

      const hintToggle = page.getByTestId('hint-toggle-0');
      await hintToggle.click();
      await expect(page.getByTestId('hint-content-0')).toBeVisible();

      await hintToggle.click();
      await expect(page.getByTestId('hint-content-0')).not.toBeVisible();
    });

    test('should navigate between steps', async ({ page }) => {
      await page.getByTestId('tab-challenges').click();

      const nextStepBtn = page.getByTestId('next-step-btn');
      await nextStepBtn.click();

      await expect(page.getByText('Step 2 of 2')).toBeVisible();

      await page.getByTestId('prev-step-btn').click();
      await expect(page.getByText('Step 1 of 2')).toBeVisible();
    });

    test('should reset code to initial state', async ({ page }) => {
      await page.getByTestId('tab-challenges').click();

      const editor = page.getByTestId('code-editor');
      const initialCode = await editor.inputValue();

      await editor.fill('modified code');
      await page.getByTestId('reset-btn').click();

      await expect(editor).toHaveValue(initialCode);
    });
  });

  test.describe('Transaction Simulation Flow', () => {
    test('should navigate to the transaction simulator', async ({ page }) => {
      await page.getByTestId('tab-simulator').click();
      await expect(page.getByTestId('sim-dashboard')).toBeVisible();
    });

    test('should select a contract and function', async ({ page }) => {
      await page.getByTestId('tab-simulator').click();

      const functionButtons = page.locator('[data-testid^="function-select-"]');
      await expect(functionButtons.first()).toBeVisible();
      await functionButtons.first().click();
    });

    test('should run a simulation and display gas metrics', async ({ page }) => {
      await page.getByTestId('tab-simulator').click();

      await page.getByTestId('run-sim-btn').click();

      await expect(page.getByTestId('simulation-content')).toBeVisible({ timeout: 5000 });
      await expect(page.getByTestId('metric-fee')).toBeVisible();
      await expect(page.getByTestId('cpu-gauge')).toBeVisible();
    });
  });

  test.describe('ABI Explorer', () => {
    test('should display the ABI explorer and contract methods', async ({ page }) => {
      await page.getByTestId('tab-abi').click();
      await expect(page.getByText('Contract ABI Explorer')).toBeVisible();
      await expect(page.getByTestId('method-increment')).toBeVisible();
    });

    test('should simulate a contract function call', async ({ page }) => {
      await page.getByTestId('tab-abi').click();

      await page.getByTestId('method-get_value').click();
      await page.getByTestId('execute-btn').click();

      await expect(page.getByTestId('execution-result')).toBeVisible({ timeout: 3000 });
    });
  });

  test.describe('Events Tab', () => {
    test('should display the live event feed', async ({ page }) => {
      await page.getByTestId('tab-events').click();
      await expect(page.getByTestId('event-feed')).toBeVisible();
      await expect(page.getByTestId('listener-status')).toBeVisible();
    });

    test('should filter events by severity', async ({ page }) => {
      await page.getByTestId('tab-events').click();

      await page.getByTestId('severity-critical').click();
      await expect(page.getByTestId('severity-critical')).toHaveClass(/active/);
    });
  });

  test.describe('Cross-browser Compatibility', () => {
    test('should render the main layout in all supported browsers', async ({ page, browserName }) => {
      await expect(page.locator('main')).toBeVisible();
      expect(['chromium', 'firefox', 'webkit']).toContain(browserName);
    });
  });

  test.describe('Performance', () => {
    test('should load the home page in under 3 seconds', async ({ page }) => {
      const startTime = Date.now();
      await page.goto('/');
      await page.waitForLoadState('networkidle');
      expect(Date.now() - startTime).toBeLessThan(3000);
    });
  });

  test.describe('Accessibility', () => {
    test('should have proper heading hierarchy', async ({ page }) => {
      const headings = page.locator('h1, h2, h3, h4, h5, h6');
      await expect(headings.first()).toBeVisible();
      expect(await headings.count()).toBeGreaterThan(0);
    });

    test('should support keyboard tab navigation', async ({ page }) => {
      await page.keyboard.press('Tab');
      const focusedTag = await page.evaluate(() => document.activeElement?.tagName);
      expect(focusedTag).toBeTruthy();
    });

    test('every tab button has accessible text', async ({ page }) => {
      const tabButtons = page.locator('.tab-btn');
      const count = await tabButtons.count();
      expect(count).toBeGreaterThan(0);

      for (let i = 0; i < count; i++) {
        const text = await tabButtons.nth(i).textContent();
        expect(text?.trim()).toBeTruthy();
      }
    });
  });

  test.describe('Responsive Layout', () => {
    for (const [name, size] of Object.entries({
      mobile: { width: 375, height: 667 },
      tablet: { width: 768, height: 1024 },
      desktop: { width: 1920, height: 1080 },
    })) {
      test(`should render the header and main content on ${name}`, async ({ page }) => {
        await page.setViewportSize(size);
        await page.goto('/');
        await expect(page.locator('.app-header')).toBeVisible();
        await expect(page.locator('main')).toBeVisible();
      });
    }
  });

  test.describe('Integration', () => {
    test('should complete a full developer workflow', async ({ page }) => {
      // Connect wallet
      await page.getByTestId('tab-wallet').click();
      await page.getByTestId('connect-freighter').click();
      await expect(page.getByTestId('connected-panel')).toBeVisible({ timeout: 5000 });

      // Write and compile a contract
      await page.getByTestId('tab-compiler').click();
      const codeTextarea = page.getByLabel('Source Code');
      await codeTextarea.fill('use soroban_sdk::{contract, contractimpl};\n#[contract]\npub struct Counter;');
      await page.getByTestId('compile-button').click();
      await expect(page.getByTestId('compiler-result')).toBeVisible();

      // Inspect the ABI
      await page.getByTestId('tab-abi').click();
      await expect(page.getByTestId('abi-testing-panel')).toBeVisible();

      // Simulate a call
      await page.getByTestId('tab-simulator').click();
      await expect(page.getByTestId('sim-dashboard')).toBeVisible();
    });

    test('should preserve compiler input across tab switches', async ({ page }) => {
      await page.getByTestId('tab-compiler').click();

      const projectInput = page.getByLabel('Project Name');
      await projectInput.fill('stateful-contract');

      await page.getByTestId('tab-metrics').click();
      await page.getByTestId('tab-compiler').click();

      await expect(projectInput).toHaveValue('stateful-contract');
    });
  });

  test.describe('App Navigation', () => {
    const tabs = [
      'tab-tutorial',
      'tab-challenges',
      'tab-events',
      'tab-simulator',
      'tab-metrics',
      'tab-multichain',
      'tab-abi',
      'tab-compiler',
      'tab-dependencies',
      'tab-wallet',
    ];

    test('should load with the Tutorial tab active', async ({ page }) => {
      await expect(page.getByTestId('tab-tutorial')).toHaveClass(/active/);
    });

    test('should display all navigation tabs', async ({ page }) => {
      for (const tabId of tabs) {
        await expect(page.getByTestId(tabId)).toBeVisible();
      }
    });

    test('should switch active tab styling on click', async ({ page }) => {
      await page.getByTestId('tab-compiler').click();
      await expect(page.getByTestId('tab-compiler')).toHaveClass(/active/);

      await page.getByTestId('tab-metrics').click();
      await expect(page.getByTestId('tab-metrics')).toHaveClass(/active/);
      await expect(page.getByTestId('tab-compiler')).not.toHaveClass(/active/);
    });
  });
});
