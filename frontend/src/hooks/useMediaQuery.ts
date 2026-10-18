import { useEffect, useState } from 'react';

/**
 * Subscribe to a CSS media query.
 *
 * Returns `false` when `matchMedia` is unavailable (server rendering, and some
 * test environments), so callers get the desktop layout rather than crashing.
 */
export function useMediaQuery(query: string): boolean {
  const getMatch = () =>
    typeof window !== 'undefined' && typeof window.matchMedia === 'function'
      ? window.matchMedia(query).matches
      : false;

  const [matches, setMatches] = useState(getMatch);

  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return;

    const list = window.matchMedia(query);
    const onChange = (event: MediaQueryListEvent) => setMatches(event.matches);

    // Re-read on subscribe: the query may have changed between render and effect.
    setMatches(list.matches);

    // addListener is deprecated but is the only API in older Safari.
    if (typeof list.addEventListener === 'function') {
      list.addEventListener('change', onChange);
      return () => list.removeEventListener('change', onChange);
    }
    list.addListener(onChange);
    return () => list.removeListener(onChange);
  }, [query]);

  return matches;
}

/** The portal's mobile breakpoint: drawer navigation below 768px. */
export const MOBILE_QUERY = '(max-width: 767px)';

/** True while the viewport is narrow enough to need drawer navigation. */
export function useIsMobile(): boolean {
  return useMediaQuery(MOBILE_QUERY);
}
