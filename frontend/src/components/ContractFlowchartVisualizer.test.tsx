import { fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { ContractFlowchartVisualizer, SAMPLE_FLOWCHART } from './ContractFlowchartVisualizer';
import { ContractFlowchartSpec } from './contractFlowchart';

const svgEl = () => screen.getByTestId('flowchart-svg').querySelector('svg');

describe('ContractFlowchartVisualizer', () => {
  it('renders an SVG diagram of the contract state machine', () => {
    render(<ContractFlowchartVisualizer />);
    const svg = svgEl();
    expect(svg).not.toBeNull();
    expect(svg?.getAttribute('viewBox')).toMatch(/^0 0 \d+/);
  });

  it('draws a node for every state in the spec', () => {
    render(<ContractFlowchartVisualizer />);
    SAMPLE_FLOWCHART.states.forEach((state) => {
      expect(svgEl()?.querySelector(`[data-state="${state}"]`)).not.toBeNull();
    });
  });

  it('starts at the spec initial state and highlights it in the diagram', () => {
    render(<ContractFlowchartVisualizer />);
    expect(screen.getByTestId('active-state')).toHaveTextContent('Uninitialized');
    expect(svgEl()?.querySelectorAll('[data-active="true"]')).toHaveLength(1);
    expect(
      svgEl()?.querySelector('[data-state="Uninitialized"]')?.getAttribute('data-active'),
    ).toBe('true');
  });

  it('lists only the transitions permitted from the active state', () => {
    render(<ContractFlowchartVisualizer />);
    expect(screen.getByTestId('transition-initialize')).toBeInTheDocument();
    expect(screen.queryByTestId('transition-release')).not.toBeInTheDocument();
  });

  it('follows a transition and moves the highlight to the target state', () => {
    render(<ContractFlowchartVisualizer />);

    fireEvent.click(screen.getByTestId('transition-initialize'));
    expect(screen.getByTestId('active-state')).toHaveTextContent('Initialized');

    fireEvent.click(screen.getByTestId('transition-fund'));
    expect(screen.getByTestId('active-state')).toHaveTextContent('Funded');
    expect(svgEl()?.querySelector('[data-state="Funded"]')?.getAttribute('data-active')).toBe('true');
  });

  it('offers every outgoing transition once the contract is funded', () => {
    render(<ContractFlowchartVisualizer initialState="Funded" />);
    expect(screen.getByTestId('transition-top_up')).toBeInTheDocument();
    expect(screen.getByTestId('transition-release')).toBeInTheDocument();
    expect(screen.getByTestId('transition-dispute')).toBeInTheDocument();
  });

  it('reports a terminal state as having no further transitions', () => {
    render(<ContractFlowchartVisualizer initialState="Released" />);
    expect(screen.getByTestId('no-transitions')).toHaveTextContent('Terminal state');
  });

  it('jumps directly to a state from the state list', () => {
    render(<ContractFlowchartVisualizer />);
    fireEvent.click(screen.getByTestId('state-Disputed'));
    expect(screen.getByTestId('active-state')).toHaveTextContent('Disputed');
    expect(screen.getByTestId('transition-resolve_for_buyer')).toBeInTheDocument();
  });

  it('marks the active state chip as pressed', () => {
    render(<ContractFlowchartVisualizer />);
    fireEvent.click(screen.getByTestId('state-Funded'));
    expect(screen.getByTestId('state-Funded')).toHaveAttribute('aria-pressed', 'true');
    expect(screen.getByTestId('state-Disputed')).toHaveAttribute('aria-pressed', 'false');
  });

  it('resets back to the initial state', () => {
    render(<ContractFlowchartVisualizer />);
    fireEvent.click(screen.getByTestId('state-Released'));
    fireEvent.click(screen.getByTestId('reset-state'));
    expect(screen.getByTestId('active-state')).toHaveTextContent('Uninitialized');
  });

  it('toggles Mermaid source reflecting the active state', () => {
    render(<ContractFlowchartVisualizer />);
    expect(screen.queryByTestId('mermaid-source')).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId('toggle-mermaid'));
    const source = screen.getByTestId('mermaid-source');
    expect(source).toHaveTextContent('stateDiagram-v2');
    expect(source).toHaveTextContent('class Uninitialized active');

    fireEvent.click(screen.getByTestId('toggle-mermaid'));
    expect(screen.queryByTestId('mermaid-source')).not.toBeInTheDocument();
  });

  it('re-renders the diagram when a custom spec is supplied', () => {
    const spec: ContractFlowchartSpec = {
      name: 'Auction',
      initial: 'Open',
      states: ['Open', 'Closed'],
      terminal: ['Closed'],
      transitions: [{ name: 'close', from: 'Open', to: 'Closed', requires: 'seller' }],
    };
    render(<ContractFlowchartVisualizer spec={spec} />);

    expect(screen.getByText('Auction lifecycle, generated from its interface')).toBeInTheDocument();
    expect(svgEl()?.querySelector('[data-state="Closed"]')).not.toBeNull();
    expect(within(screen.getByTestId('transition-close')).getByText('seller')).toBeInTheDocument();
  });
});
