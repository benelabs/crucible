import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { ContractAbiExplorer } from './ContractAbiExplorer';
import { describe, it, expect, vi } from 'vitest';

describe('ContractAbiExplorer', () => {
  it('renders default Counter contract and its functions list', () => {
    render(<ContractAbiExplorer />);

    expect(screen.getAllByText('Counter')[0]).toBeInTheDocument();
    expect(screen.getAllByText('increment')[0]).toBeInTheDocument();
    expect(screen.getAllByText('decrement')[0]).toBeInTheDocument();
    expect(screen.getAllByText('get_value')[0]).toBeInTheDocument();
    expect(screen.getAllByText('reset')[0]).toBeInTheDocument();
  });

  it('switches between contracts', () => {
    render(<ContractAbiExplorer />);

    const tokenBtn = screen.getByTestId('abi-select-token');
    fireEvent.click(tokenBtn);

    expect(screen.getAllByText('initialize')[0]).toBeInTheDocument();
    expect(screen.getAllByText('balance')[0]).toBeInTheDocument();
    expect(screen.getAllByText('transfer')[0]).toBeInTheDocument();
  });

  it('renders input fields for selected functions dynamically', () => {
    render(<ContractAbiExplorer />);

    // Select reset function on Counter
    const resetBtn = screen.getByTestId('method-reset');
    fireEvent.click(resetBtn);

    expect(screen.getByLabelText('to (u32)')).toBeInTheDocument();
  });

  it('handles execution and displays output stats', async () => {
    render(<ContractAbiExplorer />);

    // Default is Counter, increment (u32, not get_value)
    const executeBtn = screen.getByTestId('execute-btn');
    fireEvent.click(executeBtn);

    expect(executeBtn).toHaveTextContent('Executing Simulation...');
    expect(executeBtn).toBeDisabled();

    await waitFor(() => {
      expect(screen.getByTestId('execution-result')).toBeInTheDocument();
      expect(screen.getByText('Simulation Output')).toBeInTheDocument();
      expect(screen.getByText('Gas Expended')).toBeInTheDocument();
    }, { timeout: 1500 });
  });

  it('handles execution for get_value (u32 branch)', async () => {
    render(<ContractAbiExplorer />);
    fireEvent.click(screen.getByTestId('method-get_value'));
    fireEvent.click(screen.getByTestId('execute-btn'));

    await waitFor(() => {
      expect(screen.getByText('42')).toBeInTheDocument();
    }, { timeout: 1500 });
  });

  it('handles execution for deposit (u128 branch)', async () => {
    render(<ContractAbiExplorer />);
    fireEvent.click(screen.getByTestId('abi-select-vault'));
    fireEvent.click(screen.getByTestId('method-deposit'));
    
    const amountInput = screen.getByTestId('input-amount');
    fireEvent.change(amountInput, { target: { value: '1000' } });
    
    fireEvent.click(screen.getByTestId('execute-btn'));

    await waitFor(() => {
      // 1000 * 98 / 100 = 980
      expect(screen.getByText('980')).toBeInTheDocument();
    }, { timeout: 1500 });
  });

  it('handles execution for get_shares (u128 other branch)', async () => {
    render(<ContractAbiExplorer />);
    fireEvent.click(screen.getByTestId('abi-select-vault'));
    fireEvent.click(screen.getByTestId('method-get_shares'));
    
    fireEvent.click(screen.getByTestId('execute-btn'));

    await waitFor(() => {
      expect(screen.getByTestId('execution-result')).toBeInTheDocument();
    }, { timeout: 1500 });
  });

  it('handles execution for void return', async () => {
    render(<ContractAbiExplorer />);
    fireEvent.click(screen.getByTestId('method-reset'));
    fireEvent.click(screen.getByTestId('execute-btn'));

    await waitFor(() => {
      expect(screen.getAllByText('void').length).toBeGreaterThan(0);
    }, { timeout: 1500 });
  });
});
