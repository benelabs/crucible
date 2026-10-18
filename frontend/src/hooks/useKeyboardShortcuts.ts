import { useEffect } from 'react';

/**
 * A single customizable keyboard binding.
 *
 * `combo` uses `+` separated tokens, e.g. `"mod+enter"`, `"mod+k"`, `"mod+s"`,
 * `"shift+?"`. The special token `mod` matches `Cmd` on macOS and `Ctrl` on
 * every other platform (so `mod+k` covers both `Cmd+K` and `Ctrl+K`).
 */
export interface ShortcutBinding {
  id: string;
  combo: string;
  description: string;
  handler: (event: KeyboardEvent) => void;
  /** Prevent the browser default for the matched combo. Default: true. */
  preventDefault?: boolean;
  /** When false the binding is ignored. Default: true. */
  enabled?: boolean;
  /** Fire even when focus is inside an input/textarea/select. Default: false. */
  allowInInputs?: boolean;
}

export interface ShortcutOptions {
  /** Element to attach the listener to. Default: document. */
  target?: EventTarget | null;
  /** Disable the whole manager. Default: false. */
  disabled?: boolean;
}

const isMac =
  typeof navigator !== 'undefined' &&
  /mac|iphone|ipad|ipod/i.test(navigator.platform || navigator.userAgent || '');

export function isEditableTarget(target: EventTarget | null): boolean {
  if (!target || !(target as HTMLElement)?.nodeName) return false;
  const el = target as HTMLElement;
  const tag = el.nodeName.toLowerCase();
  if (tag === 'input' || tag === 'textarea' || tag === 'select') return true;
  return el.isContentEditable === true;
}

interface ParsedCombo {
  key: string;
  shift: boolean;
  alt: boolean;
  ctrl: boolean;
  meta: boolean;
  mod: boolean;
}

export function parseCombo(combo: string): ParsedCombo {
  const tokens = combo
    .toLowerCase()
    .split('+')
    .map((t) => t.trim())
    .filter(Boolean);
  const parsed: ParsedCombo = {
    key: '',
    shift: false,
    alt: false,
    ctrl: false,
    meta: false,
    mod: false,
  };
  for (const tok of tokens) {
    if (tok === 'shift') parsed.shift = true;
    else if (tok === 'alt' || tok === 'option') parsed.alt = true;
    else if (tok === 'ctrl' || tok === 'control') parsed.ctrl = true;
    else if (tok === 'meta' || tok === 'cmd' || tok === 'command' || tok === 'win')
      parsed.meta = true;
    else if (tok === 'mod') parsed.mod = true;
    else parsed.key = tok;
  }
  return parsed;
}

/** True when the keyboard event matches the parsed combo exactly. */
export function matchCombo(event: KeyboardEvent, parsed: ParsedCombo): boolean {
  const pShift = !!event.shiftKey;
  const pAlt = !!event.altKey;
  const pCtrl = !!event.ctrlKey;
  const pMeta = !!event.metaKey;

  // Shift / Alt must match exactly.
  if (parsed.shift !== pShift) return false;
  if (parsed.alt !== pAlt) return false;

  // Ctrl / Meta handling. `mod` matches Cmd (meta) on macOS or Ctrl elsewhere,
  // so either modifier satisfies it; explicit tokens are matched strictly.
  if (parsed.mod) {
    if (!(pMeta || pCtrl)) return false;
  } else if (parsed.ctrl && parsed.meta) {
    if (!pCtrl || !pMeta) return false;
  } else if (parsed.ctrl) {
    if (!pCtrl || pMeta) return false;
  } else if (parsed.meta) {
    if (!pMeta || pCtrl) return false;
  } else if (pCtrl || pMeta) {
    return false;
  }

  if (!parsed.key) return true;
  return event.key.toLowerCase() === parsed.key;
}

/**
 * Global keyboard shortcut manager. Attach an array of bindings; the manager
 * keeps a single passive keydown listener and dispatches to the first matching,
 * enabled binding.
 */
export function useKeyboardShortcuts(
  bindings: ShortcutBinding[],
  options: ShortcutOptions = {},
): void {
  const target = options.target ?? (typeof document !== 'undefined' ? document : null);

  useEffect(() => {
    if (!target) return;
    if (options.disabled) return;

    const listener = (event: Event) => {
      const kbEvent = event as KeyboardEvent;
      if (options.disabled) return;

      for (const binding of bindings) {
        if (binding.enabled === false) continue;
        const parsed = parseCombo(binding.combo);
        if (!matchCombo(kbEvent, parsed)) continue;
        if (!binding.allowInInputs && isEditableTarget(kbEvent.target)) continue;

        if (binding.preventDefault !== false) {
          kbEvent.preventDefault();
        }
        binding.handler(kbEvent);
        return;
      }
    };

    target.addEventListener('keydown', listener as EventListener);
    return () => target.removeEventListener('keydown', listener as EventListener);
  }, [bindings, options, target]);
}

export { isMac };
