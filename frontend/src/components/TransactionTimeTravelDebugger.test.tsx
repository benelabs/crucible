import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { TransactionTimeTravelDebugger } from './TransactionTimeTravelDebugger';
import { SAMPLE_TRACE } from './executionTrace';

describe('TransactionTimeTravelDebugger', () => {
  it('starts at the first frame', () => {
    render(<TransactionTimeTravelDebugger />);
    expect(screen.getByTestId('ttd-position')).toHaveTextContent(`Step 1 of ${SAMPLE_TRACE.length}`);
    expect(screen.getByTestId('ttd-operation')).toHaveTextContent('Entry — authenticate caller');
  });

  it('steps forward and renders the new frame state', () => {
    render(<TransactionTimeTravelDebugger />);
    fireEvent.click(screen.getByTestId('step-into'));

    expect(screen.getByTestId('ttd-position')).toHaveTextContent('Step 2');
    expect(screen.getByTestId('ttd-operation')).toHaveTextContent('Load escrow record from storage');
    expect(screen.getByTestId('local-amount')).toHaveTextContent('2500');
  });

  it('steps back and restores the previous frame state exactly', () => {
    render(<TransactionTimeTravelDebugger />);

    fireEvent.click(screen.getByTestId('step-into'));
    fireEvent.click(screen.getByTestId('step-into'));
    expect(screen.getByTestId('ttd-position')).toHaveTextContent('Step 3');
    expect(screen.getByTestId('local-status')).toBeInTheDocument();

    fireEvent.click(screen.getByTestId('step-back'));
    fireEvent.click(screen.getByTestId('step-back'));

    expect(screen.getByTestId('ttd-position')).toHaveTextContent('Step 1');
    expect(screen.getByTestId('ttd-operation')).toHaveTextContent('Entry — authenticate caller');
    // `status` only enters scope at frame 2, so stepping back must drop it.
    expect(screen.queryByTestId('local-status')).not.toBeInTheDocument();
  });

  it('disables Step Back at the start and forward controls at the end', () => {
    render(<TransactionTimeTravelDebugger />);
    expect(screen.getByTestId('step-back')).toBeDisabled();

    fireEvent.click(screen.getByTestId('run-to-end'));
    expect(screen.getByTestId('step-into')).toBeDisabled();
    expect(screen.getByTestId('step-over')).toBeDisabled();
    expect(screen.getByTestId('step-back')).toBeEnabled();
  });

  it('steps into a nested cross-contract call', () => {
    render(<TransactionTimeTravelDebugger />);
    fireEvent.click(screen.getByTestId('frame-2'));
    fireEvent.click(screen.getByTestId('step-into'));

    expect(screen.getByTestId('ttd-depth')).toHaveTextContent('depth 1');
    expect(screen.getByTestId('ttd-operation')).toHaveTextContent('Cross-contract call');
  });

  it('steps over a nested call without descending into it', () => {
    render(<TransactionTimeTravelDebugger />);
    fireEvent.click(screen.getByTestId('frame-2'));
    fireEvent.click(screen.getByTestId('step-over'));

    // Frames 3 and 4 are the nested transfer; stepping over lands past them.
    expect(screen.getByTestId('ttd-depth')).toHaveTextContent('depth 0');
    expect(screen.getByTestId('ttd-operation')).toHaveTextContent('Mark escrow released');
  });

  it('steps out of a nested call back to its caller', () => {
    render(<TransactionTimeTravelDebugger />);
    fireEvent.click(screen.getByTestId('frame-3'));
    expect(screen.getByTestId('ttd-depth')).toHaveTextContent('depth 1');

    fireEvent.click(screen.getByTestId('step-out'));
    expect(screen.getByTestId('ttd-depth')).toHaveTextContent('depth 0');
  });

  it('shows the storage visible at the active frame', () => {
    render(<TransactionTimeTravelDebugger />);
    expect(screen.getByTestId('storage-escrow:42:status')).toHaveTextContent('Funded');

    fireEvent.click(screen.getByTestId('frame-5'));
    expect(screen.getByTestId('storage-escrow:42:status')).toHaveTextContent('Released');
  });

  it('highlights a storage value that the step changed, with its prior value', () => {
    render(<TransactionTimeTravelDebugger />);
    fireEvent.click(screen.getByTestId('frame-5'));

    const row = screen.getByTestId('storage-escrow:42:status');
    expect(row).toHaveClass('changed');
    expect(screen.getByTestId('storage-was-escrow:42:status')).toHaveTextContent('was Funded');
  });

  it('renders the call stack of a nested frame', () => {
    render(<TransactionTimeTravelDebugger />);
    fireEvent.click(screen.getByTestId('frame-4'));

    const stack = screen.getByTestId('ttd-stack');
    expect(stack).toHaveTextContent('CESCROW.release');
    expect(stack).toHaveTextContent('CTOKEN.transfer');
  });

  it('shows events only on the frames that emit them', () => {
    render(<TransactionTimeTravelDebugger />);
    expect(screen.getByTestId('ttd-no-events')).toBeInTheDocument();

    fireEvent.click(screen.getByTestId('frame-4'));
    expect(screen.getByTestId('ttd-events')).toHaveTextContent('transfer(CESCROW, GBENEF…, 2500)');
  });

  it('scrubs to an arbitrary frame', () => {
    render(<TransactionTimeTravelDebugger />);
    fireEvent.change(screen.getByTestId('ttd-scrubber'), { target: { value: '5' } });
    expect(screen.getByTestId('ttd-position')).toHaveTextContent('Step 6');
  });

  it('restarts back to the first frame', () => {
    render(<TransactionTimeTravelDebugger />);
    fireEvent.click(screen.getByTestId('run-to-end'));
    fireEvent.click(screen.getByTestId('restart'));

    expect(screen.getByTestId('ttd-position')).toHaveTextContent('Step 1');
    expect(screen.getByTestId('restart')).toBeDisabled();
  });

  it('marks the active frame in the frame list', () => {
    render(<TransactionTimeTravelDebugger />);
    fireEvent.click(screen.getByTestId('frame-3'));
    expect(screen.getByTestId('frame-3')).toHaveAttribute('aria-current', 'step');
    expect(screen.getByTestId('frame-0')).not.toHaveAttribute('aria-current');
  });

  it('reports an empty trace rather than rendering a broken frame', () => {
    render(<TransactionTimeTravelDebugger frames={[]} />);
    expect(screen.getByTestId('ttd-empty')).toHaveTextContent('No execution trace to replay.');
  });
});
