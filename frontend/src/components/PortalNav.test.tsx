import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { PortalNav, PortalTab } from './PortalNav';

type Listener = (event: MediaQueryListEvent) => void;

const TABS: PortalTab[] = [
  { id: 'tutorial', label: 'Tutorial', icon: <span /> },
  { id: 'metrics', label: 'Gas Estimator', icon: <span /> },
  { id: 'wallet', label: 'Wallet', icon: <span /> },
];

/** Drive the viewport breakpoint; jsdom has no matchMedia of its own. */
function setViewport(isMobile: boolean) {
  const listeners = new Set<Listener>();
  let matches = isMobile;

  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    configurable: true,
    value: vi.fn().mockImplementation((media: string) => ({
      media,
      get matches() {
        return matches;
      },
      onchange: null,
      addEventListener: (_: string, cb: Listener) => listeners.add(cb),
      removeEventListener: (_: string, cb: Listener) => listeners.delete(cb),
      addListener: (cb: Listener) => listeners.add(cb),
      removeListener: (cb: Listener) => listeners.delete(cb),
      dispatchEvent: () => false,
    })),
  });

  return {
    resizeTo(next: boolean) {
      matches = next;
      listeners.forEach((cb) => cb({ matches: next } as MediaQueryListEvent));
    },
  };
}

const touch = (x: number, y: number) => ({ clientX: x, clientY: y });

afterEach(() => {
  Reflect.deleteProperty(window, 'matchMedia');
  document.body.style.overflow = '';
});

describe('PortalNav on a desktop viewport', () => {
  it('renders a horizontal tab strip with no drawer', () => {
    setViewport(false);
    render(<PortalNav tabs={TABS} activeTab="tutorial" onSelect={vi.fn()} />);

    expect(screen.getByLabelText('Dashboard views')).toBeInTheDocument();
    expect(screen.getByTestId('tab-tutorial')).toHaveClass('tab-btn');
    expect(screen.queryByTestId('nav-toggle')).not.toBeInTheDocument();
  });

  it('marks the active tab', () => {
    setViewport(false);
    render(<PortalNav tabs={TABS} activeTab="metrics" onSelect={vi.fn()} />);

    expect(screen.getByTestId('tab-metrics')).toHaveClass('active');
    expect(screen.getByTestId('tab-metrics')).toHaveAttribute('aria-current', 'page');
    expect(screen.getByTestId('tab-wallet')).not.toHaveClass('active');
  });

  it('selects a tab on click', () => {
    setViewport(false);
    const onSelect = vi.fn();
    render(<PortalNav tabs={TABS} activeTab="tutorial" onSelect={onSelect} />);

    fireEvent.click(screen.getByTestId('tab-wallet'));
    expect(onSelect).toHaveBeenCalledWith('wallet');
  });

  it('falls back to the tab strip when matchMedia is unavailable', () => {
    Reflect.deleteProperty(window, 'matchMedia');
    render(<PortalNav tabs={TABS} activeTab="tutorial" onSelect={vi.fn()} />);

    expect(screen.getByTestId('tab-tutorial')).toBeInTheDocument();
    expect(screen.queryByTestId('nav-toggle')).not.toBeInTheDocument();
  });
});

describe('PortalNav on a mobile viewport', () => {
  it('collapses into a toggle showing the active view', () => {
    setViewport(true);
    render(<PortalNav tabs={TABS} activeTab="metrics" onSelect={vi.fn()} />);

    expect(screen.getByTestId('nav-toggle')).toBeInTheDocument();
    expect(screen.getByTestId('nav-current')).toHaveTextContent('Gas Estimator');
    expect(screen.queryByTestId('nav-drawer')).not.toBeInTheDocument();
  });

  it('mounts no duplicate tab buttons while collapsed', () => {
    setViewport(true);
    render(<PortalNav tabs={TABS} activeTab="tutorial" onSelect={vi.fn()} />);
    expect(screen.queryByTestId('tab-tutorial')).not.toBeInTheDocument();
  });

  it('opens the drawer and exposes the tabs', () => {
    setViewport(true);
    render(<PortalNav tabs={TABS} activeTab="tutorial" onSelect={vi.fn()} />);

    fireEvent.click(screen.getByTestId('nav-toggle'));

    const drawer = screen.getByTestId('nav-drawer');
    expect(drawer).toHaveAttribute('role', 'dialog');
    expect(drawer).toHaveAttribute('aria-modal', 'true');
    expect(screen.getByTestId('nav-toggle')).toHaveAttribute('aria-expanded', 'true');
    TABS.forEach((tab) => expect(screen.getByTestId(`tab-${tab.id}`)).toBeInTheDocument());
  });

  it('selects a tab and closes the drawer behind it', () => {
    setViewport(true);
    const onSelect = vi.fn();
    render(<PortalNav tabs={TABS} activeTab="tutorial" onSelect={onSelect} />);

    fireEvent.click(screen.getByTestId('nav-toggle'));
    fireEvent.click(screen.getByTestId('tab-wallet'));

    expect(onSelect).toHaveBeenCalledWith('wallet');
    expect(screen.queryByTestId('nav-drawer')).not.toBeInTheDocument();
  });

  it('closes on the backdrop, the close button, and Escape', () => {
    setViewport(true);
    render(<PortalNav tabs={TABS} activeTab="tutorial" onSelect={vi.fn()} />);

    fireEvent.click(screen.getByTestId('nav-toggle'));
    fireEvent.click(screen.getByTestId('nav-backdrop'));
    expect(screen.queryByTestId('nav-drawer')).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId('nav-toggle'));
    fireEvent.click(screen.getByTestId('nav-close'));
    expect(screen.queryByTestId('nav-drawer')).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId('nav-toggle'));
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(screen.queryByTestId('nav-drawer')).not.toBeInTheDocument();
  });

  it('locks body scroll only while the drawer is open', () => {
    setViewport(true);
    render(<PortalNav tabs={TABS} activeTab="tutorial" onSelect={vi.fn()} />);

    expect(document.body.style.overflow).toBe('');
    fireEvent.click(screen.getByTestId('nav-toggle'));
    expect(document.body.style.overflow).toBe('hidden');

    fireEvent.click(screen.getByTestId('nav-close'));
    expect(document.body.style.overflow).toBe('');
  });

  it('moves focus into the drawer on open and back to the toggle on Escape', () => {
    setViewport(true);
    render(<PortalNav tabs={TABS} activeTab="tutorial" onSelect={vi.fn()} />);

    fireEvent.click(screen.getByTestId('nav-toggle'));
    expect(screen.getByTestId('nav-drawer')).toHaveFocus();

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(screen.getByTestId('nav-toggle')).toHaveFocus();
  });

  it('closes on a leftward swipe', () => {
    setViewport(true);
    render(<PortalNav tabs={TABS} activeTab="tutorial" onSelect={vi.fn()} />);

    fireEvent.click(screen.getByTestId('nav-toggle'));
    const drawer = screen.getByTestId('nav-drawer');

    fireEvent.touchStart(drawer, { touches: [touch(200, 300)] });
    fireEvent.touchEnd(drawer, { changedTouches: [touch(100, 310)] });

    expect(screen.queryByTestId('nav-drawer')).not.toBeInTheDocument();
  });

  it('ignores a short swipe and a vertical scroll', () => {
    setViewport(true);
    render(<PortalNav tabs={TABS} activeTab="tutorial" onSelect={vi.fn()} />);

    fireEvent.click(screen.getByTestId('nav-toggle'));
    const drawer = screen.getByTestId('nav-drawer');

    // Too short to count as a swipe.
    fireEvent.touchStart(drawer, { touches: [touch(200, 300)] });
    fireEvent.touchEnd(drawer, { changedTouches: [touch(170, 300)] });
    expect(screen.getByTestId('nav-drawer')).toBeInTheDocument();

    // Far enough, but mostly vertical — that is a scroll, not a dismiss.
    fireEvent.touchStart(drawer, { touches: [touch(200, 300)] });
    fireEvent.touchEnd(drawer, { changedTouches: [touch(100, 400)] });
    expect(screen.getByTestId('nav-drawer')).toBeInTheDocument();
  });

  it('does not close on a rightward swipe', () => {
    setViewport(true);
    render(<PortalNav tabs={TABS} activeTab="tutorial" onSelect={vi.fn()} />);

    fireEvent.click(screen.getByTestId('nav-toggle'));
    const drawer = screen.getByTestId('nav-drawer');

    fireEvent.touchStart(drawer, { touches: [touch(100, 300)] });
    fireEvent.touchEnd(drawer, { changedTouches: [touch(250, 300)] });
    expect(screen.getByTestId('nav-drawer')).toBeInTheDocument();
  });
});

describe('PortalNav across a viewport change', () => {
  it('swaps to the drawer when the viewport narrows', () => {
    const viewport = setViewport(false);
    render(<PortalNav tabs={TABS} activeTab="tutorial" onSelect={vi.fn()} />);
    expect(screen.getByTestId('tab-tutorial')).toHaveClass('tab-btn');

    act(() => viewport.resizeTo(true));

    expect(screen.getByTestId('nav-toggle')).toBeInTheDocument();
  });

  it('does not strand an open drawer when the viewport widens', () => {
    const viewport = setViewport(true);
    render(<PortalNav tabs={TABS} activeTab="tutorial" onSelect={vi.fn()} />);

    fireEvent.click(screen.getByTestId('nav-toggle'));
    expect(screen.getByTestId('nav-drawer')).toBeInTheDocument();

    act(() => viewport.resizeTo(false));

    expect(screen.queryByTestId('nav-drawer')).not.toBeInTheDocument();
    expect(screen.getByTestId('tab-tutorial')).toHaveClass('tab-btn');
    expect(document.body.style.overflow).toBe('');
  });
});
