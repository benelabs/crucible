import React from 'react';
import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { BlockExplorer3D } from './BlockExplorer3D';

describe('BlockExplorer3D', () => {
  it('renders the 3D WebGL explorer header and controls', () => {
    render(<BlockExplorer3D />);

    expect(screen.getByText('3D WebGL Ledger Explorer')).toBeInTheDocument();
    expect(screen.getByTestId('toggle-rotation-btn')).toBeInTheDocument();
    expect(screen.getByTestId('zoom-in-btn')).toBeInTheDocument();
    expect(screen.getByTestId('zoom-out-btn')).toBeInTheDocument();
    expect(screen.getByTestId('reset-camera-btn')).toBeInTheDocument();
  });

  it('renders canvas element and node inspector', () => {
    render(<BlockExplorer3D />);

    expect(screen.getByTestId('webgl-canvas')).toBeInTheDocument();
    expect(screen.getByTestId('node-inspector')).toBeInTheDocument();
    expect(screen.getByText('Ledger Block #1045232')).toBeInTheDocument();
    expect(screen.getByText('swap_tokens()')).toBeInTheDocument();
  });

  it('handles rotation toggle and zoom buttons click without crashing', () => {
    render(<BlockExplorer3D />);

    const rotateBtn = screen.getByTestId('toggle-rotation-btn');
    const zoomInBtn = screen.getByTestId('zoom-in-btn');
    const zoomOutBtn = screen.getByTestId('zoom-out-btn');
    const resetBtn = screen.getByTestId('reset-camera-btn');

    fireEvent.click(rotateBtn);
    fireEvent.click(zoomInBtn);
    fireEvent.click(zoomOutBtn);
    fireEvent.click(resetBtn);

    expect(screen.getByTestId('block-explorer-3d')).toBeInTheDocument();
  });
});
