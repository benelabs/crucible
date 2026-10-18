import { render, screen, fireEvent, within } from '@testing-library/react';
import { describe, it, expect } from 'vitest';
import { DependencyGraphVisualizer } from './DependencyGraphVisualizer';

describe('DependencyGraphVisualizer', () => {
  it('renders the graph header and svg canvas', () => {
    render(<DependencyGraphVisualizer />);
    expect(screen.getByText('Contract Dependency Tree Visualizer')).toBeInTheDocument();
    expect(screen.getByTestId('dep-graph-svg')).toBeInTheDocument();
    expect(screen.getByTestId('cycle-badge')).toBeInTheDocument();
  });

  it('renders a graph node for every crate and colors it by severity', () => {
    render(<DependencyGraphVisualizer />);
    const critical = screen.getByTestId('dep-node-ed25519-dalek');
    expect(critical).toBeInTheDocument();
    expect(critical).toHaveAttribute('data-severity', 'critical');
    const safe = screen.getByTestId('dep-node-crucible');
    expect(safe).toHaveAttribute('data-severity', 'none');
  });

  it('opens a detail panel when a node is clicked', () => {
    render(<DependencyGraphVisualizer />);
    expect(screen.getByTestId('dep-detail-panel')).toContainElement(
      screen.getByText(/Select a node to inspect/i),
    );

    fireEvent.click(screen.getByTestId('dep-node-soroban-sdk'));

    const panel = screen.getByTestId('dep-detail-panel');
    expect(within(panel).getByTestId('dep-detail-name')).toHaveTextContent('soroban-sdk');
    expect(within(panel).getByTestId('dep-detail-severity')).toHaveTextContent('medium');
    expect(within(panel).getByTestId('dep-detail-status')).toHaveTextContent('outdated');
    expect(within(panel).getByTestId('dep-detail-desc')).toBeInTheDocument();
    expect(within(panel).getByTestId('dep-detail-vulns')).toHaveTextContent('2');
  });

  it('updates the detail panel when a different node is selected', () => {
    render(<DependencyGraphVisualizer />);
    fireEvent.click(screen.getByTestId('dep-node-soroban-env-host'));
    expect(screen.getByTestId('dep-detail-name')).toHaveTextContent('soroban-env-host');

    fireEvent.click(screen.getByTestId('dep-node-tokio'));
    expect(screen.getByTestId('dep-detail-name')).toHaveTextContent('tokio');
    expect(screen.getByTestId('dep-detail-status')).toHaveTextContent('outdated');
  });

  it('lists the direct dependencies of the selected crate', () => {
    render(<DependencyGraphVisualizer />);
    fireEvent.click(screen.getByTestId('dep-node-crucible'));
    expect(screen.getByTestId('dep-detail-dep-soroban-sdk')).toBeInTheDocument();
    expect(screen.getByTestId('dep-detail-dep-tokio')).toBeInTheDocument();
  });

  it('re-runs the layout without crashing', () => {
    render(<DependencyGraphVisualizer />);
    fireEvent.click(screen.getByTestId('relayout-button'));
    expect(screen.getByTestId('dep-graph-svg')).toBeInTheDocument();
  });
});
