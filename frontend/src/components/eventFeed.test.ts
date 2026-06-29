import { describe, expect, it, vi } from 'vitest';
import {
  createInitialEvents,
  createNextEvent,
  filterEvents,
  formatEventTime,
} from './eventFeed';

describe('eventFeed', () => {
  it('creates deterministic initial events', () => {
    const events = createInitialEvents();

    expect(events).toHaveLength(5);
    expect(events[0]).toMatchObject({
      type: 'contract',
      displayName: 'transfer',
      ledgerClosedAt: '2026-05-31T13:40:24.000Z',
    });
    expect(events[0].topic).toEqual(['AAAADwAAAAh0cmFuc2Zlcg==', '*', '*', '**']);
  });

  it('creates the next live event from a sequence', () => {
    vi.setSystemTime(new Date('2026-05-31T14:00:00.000Z'));

    const event = createNextEvent(2, 12940250);

    expect(event.id).toBe('0000859036408882002-0000000001');
    expect(event.ledger).toBe(12940252);
    expect(event.ledgerClosedAt).toBe('2026-05-31T14:00:00.000Z');

    vi.useRealTimers();
  });

  it('filters by severity and query', () => {
    const events = createInitialEvents();

    const filtered = filterEvents(events, {
      severity: 'critical',
      query: 'admin',
    });

    expect(filtered).toHaveLength(1);
    expect(filtered[0].displayName).toBe('admin_call');
  });

  it('formats event timestamps as compact time', () => {
    const formatted = formatEventTime('2026-05-31T13:40:24.000Z');

    expect(formatted).toMatch(/\d{2}:\d{2}:\d{2}/);
  });
});
