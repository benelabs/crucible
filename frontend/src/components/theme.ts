/**
 * Theme engine shared by the toggle component and the pre-paint bootstrap
 * script in index.html. The two must agree on storage key, attribute and
 * class names, otherwise the page flashes on load.
 */

export type ThemePreference = 'light' | 'dark' | 'system';

export type ResolvedTheme = 'light' | 'dark';

export const THEME_STORAGE_KEY = 'crucible.theme';

export const THEME_PREFERENCES: ThemePreference[] = ['light', 'dark', 'system'];

const isPreference = (value: unknown): value is ThemePreference =>
  value === 'light' || value === 'dark' || value === 'system';

/** Unreadable or unrecognised storage falls back to following the system. */
export function readStoredPreference(storage: Pick<Storage, 'getItem'> = localStorage): ThemePreference {
  try {
    const stored = storage.getItem(THEME_STORAGE_KEY);
    return isPreference(stored) ? stored : 'system';
  } catch {
    return 'system';
  }
}

/** Writing never throws: private-mode storage must not break the toggle. */
export function storePreference(
  preference: ThemePreference,
  storage: Pick<Storage, 'setItem' | 'removeItem'> = localStorage,
): void {
  try {
    if (preference === 'system') {
      storage.removeItem(THEME_STORAGE_KEY);
    } else {
      storage.setItem(THEME_STORAGE_KEY, preference);
    }
  } catch {
    // Preference is still applied for this session even if it cannot persist.
  }
}

export function systemTheme(matchMediaImpl: typeof window.matchMedia | undefined = globalThis.matchMedia): ResolvedTheme {
  try {
    return matchMediaImpl?.('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
  } catch {
    return 'dark';
  }
}

export function resolveTheme(preference: ThemePreference, system: ResolvedTheme): ResolvedTheme {
  return preference === 'system' ? system : preference;
}

/**
 * Stamps the resolved theme onto the root element. `data-theme` is only set for
 * an explicit choice so that "system" keeps deferring to prefers-color-scheme,
 * while the class always reflects what is actually being shown.
 */
export function applyTheme(
  preference: ThemePreference,
  resolved: ResolvedTheme,
  root: HTMLElement = document.documentElement,
): void {
  root.classList.remove('theme-light', 'theme-dark');
  root.classList.add(`theme-${resolved}`);

  if (preference === 'system') {
    root.removeAttribute('data-theme');
  } else {
    root.setAttribute('data-theme', preference);
  }

  root.style.colorScheme = resolved;
}
