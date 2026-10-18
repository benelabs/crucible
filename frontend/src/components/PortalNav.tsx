import React, { useEffect, useRef, useState } from 'react';
import { Menu, X } from 'lucide-react';
import './PortalNav.css';
import { useIsMobile } from '../hooks/useMediaQuery';

export interface PortalTab {
  id: string;
  label: string;
  icon: React.ReactNode;
}

interface PortalNavProps {
  tabs: PortalTab[];
  activeTab: string;
  onSelect: (id: string) => void;
}

/** Horizontal distance a touch must travel before it counts as a swipe. */
const SWIPE_THRESHOLD = 60;
/** Vertical slack allowed, so a scroll is not mistaken for a swipe. */
const SWIPE_VERTICAL_TOLERANCE = 45;

/**
 * Portal navigation that collapses into a drawer below 768px.
 *
 * Only one of the two layouts is mounted at a time, so tab ids stay unique in
 * the document and there is no hidden duplicate for assistive technology to
 * announce.
 */
export const PortalNav: React.FC<PortalNavProps> = ({ tabs, activeTab, onSelect }) => {
  const isMobile = useIsMobile();
  const [open, setOpen] = useState(false);
  const drawerRef = useRef<HTMLDivElement>(null);
  const toggleRef = useRef<HTMLButtonElement>(null);
  const touchStart = useRef<{ x: number; y: number } | null>(null);

  // Leaving mobile with the drawer open would strand it open on the desktop
  // layout, where nothing can dismiss it.
  useEffect(() => {
    if (!isMobile) setOpen(false);
  }, [isMobile]);

  useEffect(() => {
    if (!open) return;

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setOpen(false);
        toggleRef.current?.focus();
      }
    };

    document.addEventListener('keydown', onKeyDown);
    drawerRef.current?.focus();

    // Prevent the page behind the drawer from scrolling under it.
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';

    return () => {
      document.removeEventListener('keydown', onKeyDown);
      document.body.style.overflow = previousOverflow;
    };
  }, [open]);

  const select = (id: string) => {
    onSelect(id);
    setOpen(false);
  };

  const onTouchStart = (event: React.TouchEvent) => {
    const touch = event.touches[0];
    touchStart.current = touch ? { x: touch.clientX, y: touch.clientY } : null;
  };

  const onTouchEnd = (event: React.TouchEvent) => {
    const start = touchStart.current;
    const touch = event.changedTouches[0];
    touchStart.current = null;
    if (!start || !touch) return;

    const dx = touch.clientX - start.x;
    const dy = Math.abs(touch.clientY - start.y);

    // A leftward swipe closes the drawer; vertical movement means the user was
    // scrolling the tab list instead.
    if (dx < -SWIPE_THRESHOLD && dy < SWIPE_VERTICAL_TOLERANCE) setOpen(false);
  };

  const tabButtons = (className: string) =>
    tabs.map((tab) => (
      <button
        key={tab.id}
        type="button"
        className={`${className} ${activeTab === tab.id ? 'active' : ''}`}
        onClick={() => select(tab.id)}
        data-testid={`tab-${tab.id}`}
        aria-current={activeTab === tab.id ? 'page' : undefined}
      >
        <span className="portal-nav-icon" aria-hidden="true">
          {tab.icon}
        </span>
        {tab.label}
      </button>
    ));

  if (!isMobile) {
    return (
      <nav className="header-tabs" aria-label="Dashboard views">
        {tabButtons('tab-btn')}
      </nav>
    );
  }

  const activeLabel = tabs.find((tab) => tab.id === activeTab)?.label ?? 'Menu';

  return (
    <div className="portal-nav-mobile">
      <button
        type="button"
        ref={toggleRef}
        className="portal-nav-toggle"
        aria-expanded={open}
        aria-controls="portal-nav-drawer"
        aria-label={open ? 'Close navigation' : 'Open navigation'}
        data-testid="nav-toggle"
        onClick={() => setOpen((v) => !v)}
      >
        <Menu size={18} />
        <span className="portal-nav-current" data-testid="nav-current">
          {activeLabel}
        </span>
      </button>

      {open && (
        <>
          <div
            className="portal-nav-backdrop"
            data-testid="nav-backdrop"
            onClick={() => setOpen(false)}
          />
          <div
            id="portal-nav-drawer"
            ref={drawerRef}
            className="portal-nav-drawer"
            role="dialog"
            aria-modal="true"
            aria-label="Dashboard views"
            tabIndex={-1}
            data-testid="nav-drawer"
            onTouchStart={onTouchStart}
            onTouchEnd={onTouchEnd}
          >
            <div className="portal-nav-drawer-head">
              <span>Views</span>
              <button
                type="button"
                className="portal-nav-close"
                aria-label="Close navigation"
                data-testid="nav-close"
                onClick={() => setOpen(false)}
              >
                <X size={18} />
              </button>
            </div>
            <nav className="portal-nav-list" aria-label="Dashboard views">
              {tabButtons('portal-nav-item')}
            </nav>
          </div>
        </>
      )}
    </div>
  );
};

export default PortalNav;
