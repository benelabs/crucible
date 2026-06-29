import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { TransactionSimulator } from './TransactionSimulator';

describe('TransactionSimulator', () => {
  it('renders correctly with default Token transfer settings', () => {
    render(<TransactionSimulator />);

    expect(screen.getByText('Transaction Simulator')).toBeInTheDocument();
    expect(screen.getByText('Dry-run smart contract calls with low-level execution insights')).toBeInTheDocument();
    expect(screen.getByTestId('contract-select-token')).toHaveClass('active');
    expect(screen.getByTestId('function-select-transfer')).toHaveClass('active');
    
    // Arguments for Token::transfer
    expect(screen.getByTestId('arg-field-from')).toBeInTheDocument();
    expect(screen.getByTestId('arg-field-to')).toBeInTheDocument();
    expect(screen.getByTestId('arg-field-amount')).toBeInTheDocument();
    
    // Check default status is Awaiting Simulation
    expect(screen.getByTestId('empty-results')).toBeInTheDocument();
    expect(screen.getByText('Awaiting Simulation')).toBeInTheDocument();
  });

  it('switches contract template and updates dynamic input fields', () => {
    render(<TransactionSimulator />);

    // Select NFT
    const nftBtn = screen.getByTestId('contract-select-nft');
    fireEvent.click(nftBtn);
    expect(nftBtn).toHaveClass('active');

    // Functions list should update
    const mintNftBtn = screen.getByTestId('function-select-mint_nft');
    const transferNftBtn = screen.getByTestId('function-select-transfer_nft');
    expect(mintNftBtn).toBeInTheDocument();
    expect(transferNftBtn).toBeInTheDocument();

    // Inputs should update to NFT arguments
    expect(screen.queryByTestId('arg-field-from')).toBeNull(); // mint_nft doesn't have from
    expect(screen.getByTestId('arg-field-to')).toBeInTheDocument();
    expect(screen.getByTestId('arg-field-tokenId')).toBeInTheDocument();
  });

  it('switches functions within the same contract', () => {
    render(<TransactionSimulator />);

    // Initially at Token::transfer. Switch to Token::mint
    const mintBtn = screen.getByTestId('function-select-mint');
    fireEvent.click(mintBtn);
    expect(mintBtn).toHaveClass('active');

    // Inputs should update (no 'from' or 'to' select; only 'to' and 'amount')
    expect(screen.queryByTestId('arg-field-from')).toBeNull();
    expect(screen.getByTestId('arg-field-to')).toBeInTheDocument();
    expect(screen.getByTestId('arg-field-amount')).toBeInTheDocument();
  });

  it('toggles the advanced configuration accordion and updates range sliders', () => {
    render(<TransactionSimulator />);

    // Accordion should be closed initially
    expect(screen.queryByTestId('env-seq-slider')).toBeNull();

    // Click header to open
    const header = screen.getByTestId('env-accordion-header');
    fireEvent.click(header);
    expect(screen.getByTestId('env-accordion')).toHaveClass('open');

    // Verify slider is rendered
    const slider = screen.getByTestId('env-seq-slider');
    expect(slider).toBeInTheDocument();

    // Change sequence number via range slider
    fireEvent.change(slider, { target: { value: '15000000' } });
    expect(screen.getByText('15,000,000')).toBeInTheDocument();
  });

  it('resets inputs back to defaults when clicking Reset Settings', () => {
    render(<TransactionSimulator />);

    // Select NFT
    fireEvent.click(screen.getByTestId('contract-select-nft'));
    expect(screen.getByTestId('contract-select-nft')).toHaveClass('active');

    // Click Reset
    const resetBtn = screen.getByTestId('reset-btn');
    fireEvent.click(resetBtn);

    // Should return to Token transfer
    expect(screen.getByTestId('contract-select-token')).toHaveClass('active');
    expect(screen.getByTestId('function-select-transfer')).toHaveClass('active');
  });

  it('simulates a successful transaction and displays detailed metrics', async () => {
    render(<TransactionSimulator />);

    const runBtn = screen.getByTestId('run-sim-btn');
    expect(runBtn).toHaveTextContent('Simulate Transaction');

    // Click to simulate
    fireEvent.click(runBtn);

    // Verify simulating states
    expect(runBtn).toHaveTextContent('Simulating Invocation...');
    expect(runBtn).toBeDisabled();
    expect(screen.getByTestId('simulating-results')).toBeInTheDocument();

    // Wait for mock VM execution (approx 900ms)
    await waitFor(() => {
      expect(runBtn).not.toBeDisabled();
      expect(screen.getByTestId('simulation-content')).toBeInTheDocument();
    }, { timeout: 1500 });

    // Verify success result elements are rendered
    const badge = screen.getByTestId('sim-status-badge');
    expect(badge).toHaveTextContent('Success');
    expect(badge).toHaveClass('success');

    // Verify numerical metrics
    expect(screen.getByTestId('metric-ram')).toHaveTextContent(/KB/);
    expect(screen.getByTestId('metric-fee')).toHaveTextContent(/Stroops/);

    // Verify CPU gauge
    const cpuGauge = screen.getByTestId('cpu-gauge');
    expect(cpuGauge).toBeInTheDocument();

    // Verify return value
    expect(screen.getByTestId('return-val')).toHaveTextContent('Result: true');

    // Verify console logs
    const consoleBox = screen.getByTestId('vm-console');
    expect(consoleBox).toBeInTheDocument();
    expect(consoleBox.textContent).toContain('Initializing Soroban Host VM Environment');
    expect(consoleBox.textContent).toContain('Execution completed successfully.');
  });

  it('simulates a reverted transaction when required signature is unchecked', async () => {
    render(<TransactionSimulator />);

    // Uncheck Alice's signature card (required for Token::transfer from Alice)
    const aliceAuthBtn = screen.getByTestId('auth-card-alice');
    expect(aliceAuthBtn).toHaveClass('checked');
    fireEvent.click(aliceAuthBtn);
    expect(aliceAuthBtn).not.toHaveClass('checked');

    // Click run simulation
    fireEvent.click(screen.getByTestId('run-sim-btn'));

    // Wait for simulation completion
    await waitFor(() => {
      expect(screen.getByTestId('simulation-content')).toBeInTheDocument();
    }, { timeout: 1500 });

    // Status badge must be Reverted
    const badge = screen.getByTestId('sim-status-badge');
    expect(badge).toHaveTextContent('Reverted');
    expect(badge).toHaveClass('reverted');

    // Error banner should be present with details
    const errorBanner = screen.getByTestId('error-banner');
    expect(errorBanner).toBeInTheDocument();
    expect(errorBanner).toHaveTextContent('ContractError::Unauthorized');
    expect(errorBanner).toHaveTextContent('Invocation lacks required signature auth for: [alice]');

    // Console logs must document failure
    const consoleBox = screen.getByTestId('vm-console');
    expect(consoleBox.textContent).toContain('Auth verification failed! Missing valid signature for account: [alice]');
  });

  it('triggers specific business logic errors such as self-transfer', async () => {
    render(<TransactionSimulator />);

    // Select Token transfer, but set 'to' to also be 'alice' to trigger self transfer
    const toField = screen.getByTestId('arg-field-to');
    fireEvent.change(toField, { target: { value: 'alice' } });

    // Click run
    fireEvent.click(screen.getByTestId('run-sim-btn'));

    // Wait for completion
    await waitFor(() => {
      expect(screen.getByTestId('simulation-content')).toBeInTheDocument();
    }, { timeout: 1500 });

    // Should fail with self-transfer error
    expect(screen.getByTestId('sim-status-badge')).toHaveTextContent('Reverted');
    expect(screen.getByTestId('error-banner')).toHaveTextContent('ContractError::SelfTransfer');
  });
});
