import { useEffect, useMemo, useState } from 'react';
import {
  type ContractEvent,
  type EventFilter,
  type ListenerStatus,
  createInitialEvents,
  createNextEvent,
  filterEvents,
} from './eventFeed';

const MAX_EVENTS = 80;
const LIVE_INTERVAL_MS = 1800;

export const useEventFeed = () => {
  const [events, setEvents] = useState<ContractEvent[]>(() => createInitialEvents());
  const [filter, setFilter] = useState<EventFilter>({ severity: 'all', query: '' });
  const [status, setStatus] = useState<ListenerStatus>('connected');
  const [sequence, setSequence] = useState(1);

  useEffect(() => {
    if (status !== 'connected') {
      return;
    }

    const intervalId = window.setInterval(() => {
      setEvents((currentEvents) => {
        const latestLedger = currentEvents[0]?.ledger ?? 12_940_250;
        return [createNextEvent(sequence, latestLedger), ...currentEvents].slice(0, MAX_EVENTS);
      });
      setSequence((currentSequence) => currentSequence + 1);
    }, LIVE_INTERVAL_MS);

    return () => window.clearInterval(intervalId);
  }, [sequence, status]);

  const visibleEvents = useMemo(() => filterEvents(events, filter), [events, filter]);

  const metrics = useMemo(() => {
    const criticalCount = events.filter((event) => event.severity === 'critical').length;
    const warningCount = events.filter((event) => event.severity === 'warning').length;
    const uniqueContracts = new Set(events.map((event) => event.contractId)).size;
    const latestLedger = events[0]?.ledger ?? 0;

    return {
      totalEvents: events.length,
      criticalCount,
      warningCount,
      uniqueContracts,
      latestLedger,
    };
  }, [events]);

  const pause = () => setStatus('paused');
  const resume = () => setStatus('connected');
  const disconnect = () => setStatus('disconnected');

  return {
    events,
    visibleEvents,
    filter,
    setFilter,
    status,
    metrics,
    pause,
    resume,
    disconnect,
  };
};
