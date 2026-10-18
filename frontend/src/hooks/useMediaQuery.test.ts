import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { MOBILE_QUERY, useIsMobile, useMediaQuery } from './useMediaQuery';

type Listener = (event: MediaQueryListEvent) => void;

/** Install a controllable matchMedia; jsdom does not provide one. */
function mockMatchMedia(initial: boolean, { legacy = false } = {}) {
  const listeners = new Set<Listener>();
  let matches = initial;

  const list = {
    get matches() {
      return matches;
    },
    media: '',
    onchange: null,
    addEventListener: legacy ? undefined : (_: string, cb: Listener) => listeners.add(cb),
    removeEventListener: legacy ? undefined : (_: string, cb: Listener) => listeners.delete(cb),
    addListener: (cb: Listener) => listeners.add(cb),
    removeListener: (cb: Listener) => listeners.delete(cb),
    dispatchEvent: () => false,
  };

  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    configurable: true,
    value: vi.fn().mockImplementation((media: string) => ({ ...list, media })),
  });

  return {
    setMatches(next: boolean) {
      matches = next;
      listeners.forEach((cb) => cb({ matches: next } as MediaQueryListEvent));
    },
    listenerCount: () => listeners.size,
  };
}

afterEach(() => {
  Reflect.deleteProperty(window, 'matchMedia');
});

describe('useMediaQuery', () => {
  it('reports the initial match', () => {
    mockMatchMedia(true);
    const { result } = renderHook(() => useMediaQuery('(max-width: 767px)'));
    expect(result.current).toBe(true);
  });

  it('updates when the viewport crosses the breakpoint', () => {
    const media = mockMatchMedia(false);
    const { result } = renderHook(() => useMediaQuery('(max-width: 767px)'));
    expect(result.current).toBe(false);

    act(() => media.setMatches(true));
    expect(result.current).toBe(true);

    act(() => media.setMatches(false));
    expect(result.current).toBe(false);
  });

  it('unsubscribes on unmount', () => {
    const media = mockMatchMedia(false);
    const { unmount } = renderHook(() => useMediaQuery('(max-width: 767px)'));
    expect(media.listenerCount()).toBe(1);

    unmount();
    expect(media.listenerCount()).toBe(0);
  });

  it('falls back to the legacy addListener API', () => {
    const media = mockMatchMedia(false, { legacy: true });
    const { result } = renderHook(() => useMediaQuery('(max-width: 767px)'));

    act(() => media.setMatches(true));
    expect(result.current).toBe(true);
  });

  it('reports false rather than throwing when matchMedia is unavailable', () => {
    Reflect.deleteProperty(window, 'matchMedia');
    const { result } = renderHook(() => useMediaQuery('(max-width: 767px)'));
    expect(result.current).toBe(false);
  });
});

describe('useIsMobile', () => {
  it('queries the 768px breakpoint', () => {
    mockMatchMedia(true);
    const { result } = renderHook(() => useIsMobile());

    expect(result.current).toBe(true);
    expect(window.matchMedia).toHaveBeenCalledWith(MOBILE_QUERY);
  });
});
