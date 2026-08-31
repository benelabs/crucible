import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import App from './App';

describe('App Component', () => {
  const originalFetch = global.fetch;

  beforeEach(() => {
    global.fetch = vi.fn();
  });

  afterEach(() => {
    global.fetch = originalFetch;
    vi.clearAllMocks();
  });
  it('renders correctly and defaults to tutorial tab', async () => {
    render(<App />);
    expect(screen.getByText('Crucible Developer Portal')).toBeInTheDocument();

    // Check that Tutorial is the active tab by default
    expect(screen.getByTestId('tab-tutorial')).toHaveClass('active');
    await waitFor(() => {
      expect(screen.getByTestId('onboarding-tutorial')).toBeInTheDocument();
    });
  });

  it('switches to Gas Estimator tab', async () => {
    render(<App />);
    const gasTabBtn = screen.getByTestId('tab-metrics');
    fireEvent.click(gasTabBtn);

    expect(gasTabBtn).toHaveClass('active');
    // Gas estimator component is lazy-loaded
    await waitFor(() => {
      expect(screen.getByText('Gas Cost Estimator')).toBeInTheDocument();
    });
  });

  it('switches to MultiChain Dashboard tab', async () => {
    render(<App />);
    const multiChainBtn = screen.getByTestId('tab-multichain');
    fireEvent.click(multiChainBtn);

    expect(multiChainBtn).toHaveClass('active');
    await waitFor(() => {
      expect(screen.getByText('Multi-Chain Support')).toBeInTheDocument();
    });
  });

  it('switches to ABI Explorer tab', async () => {
    render(<App />);
    const abiBtn = screen.getByTestId('tab-abi');
    fireEvent.click(abiBtn);

    expect(abiBtn).toHaveClass('active');
    await waitFor(() => {
      expect(screen.getByText('Contract ABI Explorer')).toBeInTheDocument();
    });
  });

  it('switches to Challenges tab', async () => {
    render(<App />);
    const challengesBtn = screen.getByTestId('tab-challenges');
    fireEvent.click(challengesBtn);

    expect(challengesBtn).toHaveClass('active');
    await waitFor(() => {
      expect(screen.getByTestId('challenge-engine')).toBeInTheDocument();
    });
  });

  it('switches to Compiler Service tab', () => {
    render(<App />);
    const compilerBtn = screen.getByTestId('tab-compiler');
    fireEvent.click(compilerBtn);
    
    expect(compilerBtn).toHaveClass('active');
    expect(screen.getByText('On-Demand compilation service')).toBeInTheDocument();
  });

  it('switches to Dependency Analyzer tab', () => {
    render(<App />);
    const depBtn = screen.getByTestId('tab-dependencies');
    fireEvent.click(depBtn);
    
    expect(depBtn).toHaveClass('active');
    expect(screen.getByText('Cargo Dependency Analyzer')).toBeInTheDocument();
  });

  it('handles compile success', async () => {
    (global.fetch as any).mockResolvedValueOnce({
      json: async () => ({ status: 'success', data: { status: 'success', wasmSizeBytes: 1024, compileTimeMs: 150, wasmHash: 'abc', logs: 'Compiled' } })
    });

    render(<App />);
    fireEvent.click(screen.getByTestId('tab-compiler'));
    
    const compileBtn = screen.getByTestId('compile-button');
    fireEvent.click(compileBtn);
    
    await waitFor(() => {
      expect(screen.getByText('Size: 1024 B')).toBeInTheDocument();
      expect(screen.getByText('Compiled')).toBeInTheDocument();
    });
  });

  it('handles compile error', async () => {
    (global.fetch as any).mockRejectedValueOnce(new Error('Network error'));

    render(<App />);
    fireEvent.click(screen.getByTestId('tab-compiler'));
    
    const compileBtn = screen.getByTestId('compile-button');
    fireEvent.click(compileBtn);
    
    await waitFor(() => {
      expect(screen.getByText('Connection error: Network error')).toBeInTheDocument();
    });
  });

  it('handles analysis success', async () => {
    (global.fetch as any).mockResolvedValueOnce({
      json: async () => ({ status: 'success', data: { cyclesDetected: false, vulnerabilityCount: 0, dependencies: [{ name: 'serde', version: '1.0', status: 'up-to-date' }] } })
    });

    render(<App />);
    fireEvent.click(screen.getByTestId('tab-dependencies'));
    
    const analyzeBtn = screen.getByTestId('analyze-button');
    fireEvent.click(analyzeBtn);
    
    await waitFor(() => {
      expect(screen.getByText('serde')).toBeInTheDocument();
      expect(screen.getByText('up-to-date')).toBeInTheDocument();
    });
  });

  it('handles analysis error', async () => {
    (global.fetch as any).mockRejectedValueOnce(new Error('Network error'));

    render(<App />);
    fireEvent.click(screen.getByTestId('tab-dependencies'));
    
    const analyzeBtn = screen.getByTestId('analyze-button');
    fireEvent.click(analyzeBtn);
    
    await waitFor(() => {
      expect(screen.queryByText(/Load cargo descriptor file/)).not.toBeInTheDocument();
    });
  });
});
