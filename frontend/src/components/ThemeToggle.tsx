import React, { useCallback, useEffect, useState } from 'react';
import { Monitor, Moon, Sun } from 'lucide-react';
import {
  THEME_PREFERENCES,
  applyTheme,
  readStoredPreference,
  resolveTheme,
  storePreference,
  systemTheme,
  type ResolvedTheme,
  type ThemePreference,
} from './theme';
import './ThemeToggle.css';

const PREFERENCE_META: Record<ThemePreference, { label: string; icon: React.ReactNode }> = {
  light: { label: 'Light', icon: <Sun size={14} /> },
  dark: { label: 'Dark', icon: <Moon size={14} /> },
  system: { label: 'System', icon: <Monitor size={14} /> },
};

export interface UseThemeResult {
  preference: ThemePreference;
  resolved: ResolvedTheme;
  setPreference: (preference: ThemePreference) => void;
}

/**
 * Owns the theme preference, keeps it applied to <html>, and follows the OS
 * while the preference is "system".
 */
export function useTheme(): UseThemeResult {
  const [preference, setPreferenceState] = useState<ThemePreference>(() => readStoredPreference());
  const [system, setSystem] = useState<ResolvedTheme>(() => systemTheme());

  // Track the OS setting; it only changes the rendered theme while on "system".
  useEffect(() => {
    const query = globalThis.matchMedia?.('(prefers-color-scheme: light)');
    if (!query) return undefined;

    const handleChange = (event: MediaQueryListEvent) => setSystem(event.matches ? 'light' : 'dark');
    query.addEventListener('change', handleChange);
    return () => query.removeEventListener('change', handleChange);
  }, []);

  const resolved = resolveTheme(preference, system);

  useEffect(() => {
    applyTheme(preference, resolved);
  }, [preference, resolved]);

  const setPreference = useCallback((next: ThemePreference) => {
    storePreference(next);
    setPreferenceState(next);
  }, []);

  return { preference, resolved, setPreference };
}

export const ThemeToggle: React.FC = () => {
  const { preference, resolved, setPreference } = useTheme();

  return (
    <div
      className="theme-toggle"
      role="group"
      aria-label="Colour theme"
      data-resolved-theme={resolved}
      data-testid="theme-toggle"
    >
      {THEME_PREFERENCES.map((option) => (
        <button
          key={option}
          type="button"
          className={`theme-toggle-btn ${preference === option ? 'active' : ''}`}
          onClick={() => setPreference(option)}
          aria-pressed={preference === option}
          title={`${PREFERENCE_META[option].label} theme`}
          data-testid={`theme-${option}`}
        >
          {PREFERENCE_META[option].icon}
          <span className="theme-toggle-label">{PREFERENCE_META[option].label}</span>
        </button>
      ))}
    </div>
  );
};
