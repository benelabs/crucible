import { render, fireEvent, screen } from '@testing-library/react';
import { describe, it, expect, vi } from 'vitest';
import {
  useKeyboardShortcuts,
  parseCombo,
  matchCombo,
  type ShortcutBinding,
} from './useKeyboardShortcuts';

function Harness({
  bindings,
  disabled,
}: {
  bindings: ShortcutBinding[];
  disabled?: boolean;
}) {
  useKeyboardShortcuts(bindings, { disabled });
  return <div data-testid="harness">harness</div>;
}

describe('parseCombo / matchCombo', () => {
  it('parses mod+k into a meta/ctrl requirement', () => {
    const p = parseCombo('mod+k');
    expect(p.key).toBe('k');
    expect(p.mod).toBe(true);
    expect(p.shift).toBe(false);
  });

  it('matches mod+k with meta or ctrl pressed', () => {
    const p = parseCombo('mod+k');
    expect(matchCombo({ key: 'k', metaKey: true } as KeyboardEvent, p)).toBe(true);
    expect(matchCombo({ key: 'k', ctrlKey: true } as KeyboardEvent, p)).toBe(true);
    expect(matchCombo({ key: 'k', shiftKey: true } as KeyboardEvent, p)).toBe(false);
  });

  it('is strict about extra modifiers', () => {
    const p = parseCombo('mod+enter');
    expect(
      matchCombo({ key: 'Enter', metaKey: true, shiftKey: true } as KeyboardEvent, p),
    ).toBe(false);
    expect(matchCombo({ key: 'Enter', metaKey: true } as KeyboardEvent, p)).toBe(true);
  });
});

describe('useKeyboardShortcuts', () => {
  it('fires the handler on the exact key combination', () => {
    const handler = vi.fn();
    render(
      <Harness
        bindings={[
          {
            id: 'palette',
            combo: 'mod+k',
            description: 'open palette',
            handler,
          },
        ]}
      />,
    );
    fireEvent.keyDown(document, { key: 'k', metaKey: true });
    expect(handler).toHaveBeenCalledTimes(1);
  });

  it('calls preventDefault by default', () => {
    const handler = vi.fn();
    render(
      <Harness
        bindings={[{ id: 'x', combo: 'mod+s', description: 'save', handler }]}
      />,
    );
    const evt = new KeyboardEvent('keydown', { key: 's', metaKey: true, cancelable: true });
    document.dispatchEvent(evt);
    expect(handler).toHaveBeenCalled();
    expect(evt.defaultPrevented).toBe(true);
  });

  it('does not fire for a non-matching combination', () => {
    const handler = vi.fn();
    render(
      <Harness
        bindings={[{ id: 'x', combo: 'mod+k', description: 'open', handler }]}
      />,
    );
    fireEvent.keyDown(document, { key: 'k', ctrlKey: true, shiftKey: true });
    expect(handler).not.toHaveBeenCalled();
  });

  it('matches ctrl when mod is used (cross-platform)', () => {
    const handler = vi.fn();
    render(
      <Harness
        bindings={[{ id: 'x', combo: 'mod+enter', description: 'run', handler }]}
      />,
    );
    fireEvent.keyDown(document, { key: 'Enter', ctrlKey: true });
    expect(handler).toHaveBeenCalledTimes(1);
  });

  it('ignores disabled bindings', () => {
    const handler = vi.fn();
    render(
      <Harness
        bindings={[
          { id: 'x', combo: 'mod+k', description: 'open', handler, enabled: false },
        ]}
      />,
    );
    fireEvent.keyDown(document, { key: 'k', metaKey: true });
    expect(handler).not.toHaveBeenCalled();
  });

  it('skips bindings while typing in an input unless allowInInputs', () => {
    const blockedHandler = vi.fn();
    const allowedHandler = vi.fn();
    render(
      <div>
        <input data-testid="field" />
        <Harness
          bindings={[
            { id: 'a', combo: 'mod+k', description: 'blocked', handler: blockedHandler },
            {
              id: 'b',
              combo: 'mod+/',
              description: 'allowed',
              handler: allowedHandler,
              allowInInputs: true,
            },
          ]}
        />
      </div>,
    );
    const field = screen.getByTestId('field');
    fireEvent.keyDown(field, { key: 'k', metaKey: true });
    expect(blockedHandler).not.toHaveBeenCalled();
    fireEvent.keyDown(field, { key: '/', metaKey: true });
    expect(allowedHandler).toHaveBeenCalledTimes(1);
  });

  it('disables the whole manager when disabled is true', () => {
    const handler = vi.fn();
    render(
      <Harness
        bindings={[{ id: 'x', combo: 'mod+k', description: 'open', handler }]}
        disabled
      />,
    );
    fireEvent.keyDown(document, { key: 'k', metaKey: true });
    expect(handler).not.toHaveBeenCalled();
  });

  it('supports arrow/escape navigation combos', () => {
    const up = vi.fn();
    const esc = vi.fn();
    render(
      <Harness
        bindings={[
          { id: 'up', combo: 'arrowup', description: 'up', handler: up, allowInInputs: true },
          { id: 'esc', combo: 'escape', description: 'esc', handler: esc, allowInInputs: true },
        ]}
      />,
    );
    fireEvent.keyDown(document, { key: 'ArrowUp' });
    expect(up).toHaveBeenCalledTimes(1);
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(esc).toHaveBeenCalledTimes(1);
  });
});
