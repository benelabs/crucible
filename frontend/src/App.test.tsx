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
  it('renders correctly and defaults to tutorial tab', () => {
    render(<App />);
    expect(screen.getByText('Crucible Developer Portal')).toBeInTheDocument();
    
    // Check that Tutorial is the active tab by default
    expect(screen.getByTestId('tab-tutorial')).toHaveClass('active');
    expect(screen.getByTestId('onboarding-tutorial')).toBeInTheDocument();
  });

  it('switches to Gas Estimator tab', () => {
    render(<App />);
    const gasTabBtn = screen.getByTestId('tab-metrics');
    fireEvent.click(gasTabBtn);
    
    expect(gasTabBtn).toHaveClass('active');
    // Gas estimator component should be in document
    expect(screen.getByText('Gas Cost Estimator')).toBeInTheDocument();
  });

  it('switches to MultiChain Dashboard tab', () => {
    render(<App />);
    const multiChainBtn = screen.getByTestId('tab-multichain');
    fireEvent.click(multiChainBtn);
    
    expect(multiChainBtn).toHaveClass('active');
    expect(screen.getByText('Multi-Chain Support')).toBeInTheDocument();
  });

  it('switches to ABI Explorer tab', () => {
    render(<App />);
    const abiBtn = screen.getByTestId('tab-abi');
    fireEvent.click(abiBtn);
    
    expect(abiBtn).toHaveClass('active');
    expect(screen.getByText('Contract ABI Explorer')).toBeInTheDocument();
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

  it('opens the Dependency Graph tab and renders the visualizer', () => {
    render(<App />);
    const graphBtn = screen.getByTestId('tab-graph');
    fireEvent.click(graphBtn);
    expect(graphBtn).toHaveClass('active');
    expect(screen.getByText('Contract Dependency Tree Visualizer')).toBeInTheDocument();
  });

  it('triggers compile via the Cmd/Ctrl+Enter shortcut', async () => {
    (global.fetch as any).mockResolvedValueOnce({
      json: async () => ({ status: 'success', data: { status: 'success', wasmSizeBytes: 10, compileTimeMs: 1, wasmHash: 'x', logs: 'ok' } }),
    });
    render(<App />);
    fireEvent.click(screen.getByTestId('tab-compiler'));
    fireEvent.keyDown(document, { key: 'Enter', metaKey: true });
    await waitFor(() => {
      expect(screen.getByText('Size: 10 B')).toBeInTheDocument();
    });
  });

  it('saves a draft via the Cmd/Ctrl+S shortcut', () => {
    render(<App />);
    fireEvent.keyDown(document, { key: 's', metaKey: true });
    expect(localStorage.getItem('crucible:compile-draft')).not.toBeNull();
    expect(localStorage.getItem('crucible:draft-saved-at')).not.toBeNull();
  });

  it('opens the command palette via the Cmd/Ctrl+K shortcut', () => {
    render(<App />);
    expect(screen.queryByTestId('command-palette-modal')).not.toBeInTheDocument();
    fireEvent.keyDown(document, { key: 'k', metaKey: true });
    expect(screen.getByTestId('command-palette-modal')).toBeInTheDocument();
  });
});
