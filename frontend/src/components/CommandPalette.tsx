import React, { useEffect, useMemo, useRef, useState } from 'react';
import { Search, CornerDownLeft, ArrowUp, ArrowDown } from 'lucide-react';
import { useKeyboardShortcuts } from '../hooks/useKeyboardShortcuts';
import './CommandPalette.css';

export interface PaletteCommand {
  id: string;
  title: string;
  subtitle?: string;
  group?: string;
  shortcut?: string;
  run: () => void;
}

interface CommandPaletteProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  commands: PaletteCommand[];
  placeholder?: string;
}

export const CommandPalette: React.FC<CommandPaletteProps> = ({
  open,
  onOpenChange,
  commands,
  placeholder = 'Search commands…',
}) => {
  if (!open) return null;
  // Mounted only while open: local state resets on every open.
  return (
    <CommandPaletteModal
      onOpenChange={onOpenChange}
      commands={commands}
      placeholder={placeholder}
    />
  );
};

const CommandPaletteModal: React.FC<Omit<CommandPaletteProps, 'open'>> = ({
  onOpenChange,
  commands,
  placeholder,
}) => {
  const [query, setQuery] = useState('');
  const [activeIndex, setActiveIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return commands;
    return commands.filter((c) => {
      const hay = `${c.title} ${c.subtitle ?? ''} ${c.group ?? ''}`.toLowerCase();
      return hay.includes(q);
    });
  }, [commands, query]);

  const safeIndex = Math.min(activeIndex, Math.max(filtered.length - 1, 0));

  const runCommand = (cmd: PaletteCommand | undefined) => {
    if (!cmd) return;
    cmd.run();
    onOpenChange(false);
  };

  // Internal navigation shortcuts (only active while the palette is mounted).
  useKeyboardShortcuts([
    {
      id: 'palette-close',
      combo: 'escape',
      description: 'Close the command palette',
      allowInInputs: true,
      handler: () => onOpenChange(false),
    },
    {
      id: 'palette-next',
      combo: 'arrowdown',
      description: 'Move selection down',
      allowInInputs: true,
      handler: () => setActiveIndex((i) => Math.min(i + 1, filtered.length - 1)),
    },
    {
      id: 'palette-prev',
      combo: 'arrowup',
      description: 'Move selection up',
      allowInInputs: true,
      handler: () => setActiveIndex((i) => Math.max(i - 1, 0)),
    },
    {
      id: 'palette-run',
      combo: 'enter',
      description: 'Run the selected command',
      allowInInputs: true,
      handler: () => runCommand(filtered[safeIndex]),
    },
  ]);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  return (
    <div
      className="command-palette-overlay"
      data-testid="command-palette-overlay"
      onClick={(e) => {
        if (e.target === e.currentTarget) onOpenChange(false);
      }}
    >
      <div
        className="command-palette-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Command palette"
        data-testid="command-palette-modal"
      >
        <div className="command-palette-input-row">
          <Search size={16} className="command-palette-search-icon" />
          <input
            ref={inputRef}
            type="text"
            className="command-palette-input"
            placeholder={placeholder}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setActiveIndex(0);
            }}
            data-testid="command-palette-input"
            aria-label="Command search"
          />
          <kbd className="command-palette-kbd">esc</kbd>
        </div>

        <ul className="command-palette-list" data-testid="command-palette-list">
          {filtered.length === 0 && (
            <li className="command-palette-empty" data-testid="command-palette-empty">
              No matching commands
            </li>
          )}
          {filtered.map((cmd, i) => (
            <li
              key={cmd.id}
              className={`command-palette-item ${i === safeIndex ? 'active' : ''}`}
              data-testid={`command-palette-item-${cmd.id}`}
              data-index={i}
              onMouseEnter={() => setActiveIndex(i)}
              onClick={() => runCommand(cmd)}
            >
              <div className="command-palette-item-main">
                <span className="command-palette-item-title">{cmd.title}</span>
                {cmd.subtitle && (
                  <span className="command-palette-item-subtitle">{cmd.subtitle}</span>
                )}
              </div>
              {cmd.shortcut && (
                <kbd className="command-palette-kbd">{cmd.shortcut}</kbd>
              )}
            </li>
          ))}
        </ul>

        <div className="command-palette-footer">
          <span>
            <ArrowUp size={12} /> <ArrowDown size={12} /> navigate
          </span>
          <span>
            <CornerDownLeft size={12} /> run
          </span>
        </div>
      </div>
    </div>
  );
};

export default CommandPalette;
