import { render, screen, fireEvent, act } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ThemeToggle, useTheme } from './ThemeToggle';
import {
  THEME_STORAGE_KEY,
  applyTheme,
  readStoredPreference,
  resolveTheme,
  storePreference,
  systemTheme,
} from './theme';

/** Minimal matchMedia stub whose match state can be flipped mid-test. */
const stubMatchMedia = (prefersLight: boolean) => {
  const listeners = new Set<(event: MediaQueryListEvent) => void>();
  const query = {
    matches: prefersLight,
    addEventListener: (_: string, listener: (event: MediaQueryListEvent) => void) => {
      listeners.add(listener);
    },
    removeEventListener: (_: string, listener: (event: MediaQueryListEvent) => void) => {
      listeners.delete(listener);
    },
  };
  const impl = vi.fn(() => query) as unknown as typeof window.matchMedia;
  globalThis.matchMedia = impl;
  return {
    impl,
    emit(matches: boolean) {
      query.matches = matches;
      listeners.forEach((listener) => listener({ matches } as MediaQueryListEvent));
    },
  };
};

const originalMatchMedia = globalThis.matchMedia;

afterEach(() => {
  globalThis.matchMedia = originalMatchMedia;
  document.documentElement.className = '';
  document.documentElement.removeAttribute('data-theme');
  document.documentElement.style.colorScheme = '';
});

describe('readStoredPreference', () => {
  it('defaults to system when nothing is stored', () => {
    expect(readStoredPreference()).toBe('system');
  });

  it('reads a stored preference', () => {
    localStorage.setItem(THEME_STORAGE_KEY, 'light');
    expect(readStoredPreference()).toBe('light');
  });

  it('ignores an unrecognised stored value', () => {
    localStorage.setItem(THEME_STORAGE_KEY, 'neon');
    expect(readStoredPreference()).toBe('system');
  });

  it('falls back to system when storage throws', () => {
    const storage = {
      getItem: () => {
        throw new Error('blocked');
      },
    };
    expect(readStoredPreference(storage)).toBe('system');
  });
});

describe('storePreference', () => {
  it('persists an explicit choice and clears it for system', () => {
    storePreference('dark');
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe('dark');

    storePreference('system');
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBeNull();
  });

  it('never throws when storage is unavailable', () => {
    const storage = {
      setItem: () => {
        throw new Error('blocked');
      },
      removeItem: () => {
        throw new Error('blocked');
      },
    };
    expect(() => storePreference('dark', storage)).not.toThrow();
  });
});

describe('systemTheme', () => {
  it('reports light only when the media query matches', () => {
    expect(systemTheme(vi.fn(() => ({ matches: true })) as never)).toBe('light');
    expect(systemTheme(vi.fn(() => ({ matches: false })) as never)).toBe('dark');
  });

  it('falls back to dark without matchMedia support', () => {
    expect(systemTheme(undefined)).toBe('dark');
  });
});

describe('resolveTheme', () => {
  it('follows the system only for the system preference', () => {
    expect(resolveTheme('system', 'light')).toBe('light');
    expect(resolveTheme('system', 'dark')).toBe('dark');
    expect(resolveTheme('light', 'dark')).toBe('light');
    expect(resolveTheme('dark', 'light')).toBe('dark');
  });
});

describe('applyTheme', () => {
  it('toggles the theme class on the root element', () => {
    const root = document.createElement('html');

    applyTheme('dark', 'dark', root);
    expect(root.classList.contains('theme-dark')).toBe(true);
    expect(root.classList.contains('theme-light')).toBe(false);

    applyTheme('light', 'light', root);
    expect(root.classList.contains('theme-light')).toBe(true);
    expect(root.classList.contains('theme-dark')).toBe(false);
  });

  it('stamps data-theme only for an explicit preference', () => {
    const root = document.createElement('html');

    applyTheme('light', 'light', root);
    expect(root.getAttribute('data-theme')).toBe('light');

    applyTheme('system', 'dark', root);
    expect(root.hasAttribute('data-theme')).toBe(false);
    // The class still reflects what is actually rendered.
    expect(root.classList.contains('theme-dark')).toBe(true);
  });

  it('sets color-scheme so native controls follow the theme', () => {
    const root = document.createElement('html');

    applyTheme('dark', 'dark', root);
    expect(root.style.colorScheme).toBe('dark');
  });
});

describe('ThemeToggle', () => {
  beforeEach(() => {
    stubMatchMedia(false);
  });

  it('starts on system and applies the resolved dark class', () => {
    render(<ThemeToggle />);

    expect(screen.getByTestId('theme-system')).toHaveAttribute('aria-pressed', 'true');
    expect(document.documentElement.classList.contains('theme-dark')).toBe(true);
    expect(document.documentElement.hasAttribute('data-theme')).toBe(false);
  });

  it('toggles the root class when an explicit theme is chosen', () => {
    render(<ThemeToggle />);

    fireEvent.click(screen.getByTestId('theme-light'));

    expect(document.documentElement.classList.contains('theme-light')).toBe(true);
    expect(document.documentElement.classList.contains('theme-dark')).toBe(false);
    expect(document.documentElement.getAttribute('data-theme')).toBe('light');
    expect(screen.getByTestId('theme-toggle')).toHaveAttribute('data-resolved-theme', 'light');
  });

  it('persists the chosen preference', () => {
    render(<ThemeToggle />);

    fireEvent.click(screen.getByTestId('theme-dark'));
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe('dark');

    fireEvent.click(screen.getByTestId('theme-system'));
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBeNull();
  });

  it('restores a persisted preference on mount', () => {
    localStorage.setItem(THEME_STORAGE_KEY, 'light');

    render(<ThemeToggle />);

    expect(screen.getByTestId('theme-light')).toHaveAttribute('aria-pressed', 'true');
    expect(document.documentElement.classList.contains('theme-light')).toBe(true);
  });

  it('follows OS changes while on system', () => {
    const media = stubMatchMedia(false);
    render(<ThemeToggle />);
    expect(document.documentElement.classList.contains('theme-dark')).toBe(true);

    act(() => media.emit(true));

    expect(document.documentElement.classList.contains('theme-light')).toBe(true);
    expect(document.documentElement.hasAttribute('data-theme')).toBe(false);
  });

  it('ignores OS changes once an explicit theme is chosen', () => {
    const media = stubMatchMedia(false);
    render(<ThemeToggle />);

    fireEvent.click(screen.getByTestId('theme-dark'));
    act(() => media.emit(true));

    expect(document.documentElement.classList.contains('theme-dark')).toBe(true);
  });
});

describe('useTheme', () => {
  it('exposes the resolved theme for the current preference', () => {
    stubMatchMedia(true);
    let result: ReturnType<typeof useTheme> | null = null;

    const Probe = () => {
      result = useTheme();
      return null;
    };
    render(<Probe />);

    expect(result!.preference).toBe('system');
    expect(result!.resolved).toBe('light');
  });
});
