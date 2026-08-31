import { render, screen, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import { useState } from 'react';
import { CommandPalette, PaletteCommand } from './CommandPalette';

const SAMPLE_COMMANDS: PaletteCommand[] = [
  { id: 'compile', title: 'Compile Source', subtitle: 'Build WASM', shortcut: '⌘↵', run: () => {} },
  { id: 'save', title: 'Save Draft', subtitle: 'Persist editor', shortcut: '⌘S', run: () => {} },
  { id: 'graph', title: 'Open Dependency Graph', group: 'Navigate', run: () => {} },
  { id: 'wallet', title: 'Open Wallet', group: 'Navigate', run: () => {} },
];

function Harness({ initialOpen = true }: { initialOpen?: boolean }) {
  const [open, setOpen] = useState(initialOpen);
  return (
    <>
      <button data-testid="toggle" onClick={() => setOpen((o) => !o)}>
        toggle
      </button>
      <CommandPalette open={open} onOpenChange={setOpen} commands={SAMPLE_COMMANDS} />
    </>
  );
}

describe('CommandPalette', () => {
  it('does not render when closed', () => {
    render(<Harness initialOpen={false} />);
    expect(screen.queryByTestId('command-palette-modal')).not.toBeInTheDocument();
  });

  it('renders the modal and all commands when open', () => {
    render(<Harness />);
    expect(screen.getByTestId('command-palette-modal')).toBeInTheDocument();
    expect(screen.getByTestId('command-palette-item-compile')).toBeInTheDocument();
    expect(screen.getByTestId('command-palette-item-wallet')).toBeInTheDocument();
  });

  it('filters commands by query', () => {
    render(<Harness />);
    fireEvent.change(screen.getByTestId('command-palette-input'), {
      target: { value: 'wallet' },
    });
    expect(screen.getByTestId('command-palette-item-wallet')).toBeInTheDocument();
    expect(screen.queryByTestId('command-palette-item-compile')).not.toBeInTheDocument();
  });

  it('shows an empty state when nothing matches', () => {
    render(<Harness />);
    fireEvent.change(screen.getByTestId('command-palette-input'), {
      target: { value: 'zzzzz' },
    });
    expect(screen.getByTestId('command-palette-empty')).toBeInTheDocument();
  });

  it('runs the active command on Enter', () => {
    const commands = SAMPLE_COMMANDS.map((c) => ({ ...c, run: vi.fn() }));
    function Local() {
      const [open, setOpen] = useState(true);
      return <CommandPalette open={open} onOpenChange={setOpen} commands={commands} />;
    }
    render(<Local />);
    fireEvent.keyDown(document, { key: 'Enter' });
    expect(commands[0].run).toHaveBeenCalledTimes(1);
  });

  it('navigates with arrow keys and runs the highlighted command', () => {
    const commands = SAMPLE_COMMANDS.map((c) => ({ ...c, run: vi.fn() }));
    function Local() {
      const [open, setOpen] = useState(true);
      return <CommandPalette open={open} onOpenChange={setOpen} commands={commands} />;
    }
    render(<Local />);
    fireEvent.keyDown(document, { key: 'ArrowDown' });
    fireEvent.keyDown(document, { key: 'ArrowDown' });
    fireEvent.keyDown(document, { key: 'Enter' });
    expect(commands[2].run).toHaveBeenCalledTimes(1);
  });

  it('runs a command when its row is clicked and closes the palette', () => {
    const commands = SAMPLE_COMMANDS.map((c) => ({ ...c, run: vi.fn() }));
    function Local() {
      const [open, setOpen] = useState(true);
      return (
        <>
          <span data-testid="outside">outside</span>
          <CommandPalette open={open} onOpenChange={setOpen} commands={commands} />
        </>
      );
    }
    render(<Local />);
    fireEvent.click(screen.getByTestId('command-palette-item-graph'));
    expect(commands[2].run).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('command-palette-modal')).not.toBeInTheDocument();
  });

  it('closes on Escape', () => {
    render(<Harness />);
    expect(screen.getByTestId('command-palette-modal')).toBeInTheDocument();
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(screen.queryByTestId('command-palette-modal')).not.toBeInTheDocument();
  });
});
