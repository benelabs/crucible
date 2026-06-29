import React, { useMemo, useState } from 'react';
import {
  AlertTriangle,
  Bell,
  CirclePause,
  CirclePlay,
  Clock3,
  Filter,
  Radio,
  Search,
  Server,
  ShieldAlert,
  Unplug,
} from 'lucide-react';
import { formatEventTime, type ContractEvent, type EventSeverity } from './eventFeed';
import { useEventFeed } from './useEventFeed';
import './EventListenerDashboard.css';

const severityOptions: Array<'all' | EventSeverity> = [
  'all',
  'info',
  'success',
  'warning',
  'critical',
];

const severityLabels: Record<'all' | EventSeverity, string> = {
  all: 'All',
  info: 'Info',
  success: 'Success',
  warning: 'Warning',
  critical: 'Critical',
};

export const EventListenerDashboard: React.FC = () => {
  const {
    visibleEvents,
    filter,
    setFilter,
    status,
    metrics,
    pause,
    resume,
    disconnect,
  } = useEventFeed();
  const [selectedEventId, setSelectedEventId] = useState<string | null>(null);

  const selectedEvent = useMemo(() => {
    return (
      visibleEvents.find((event) => event.id === selectedEventId) ??
      visibleEvents[0] ??
      null
    );
  }, [selectedEventId, visibleEvents]);

  return (
    <section className="event-dashboard" aria-label="Event listener dashboard">
      <div className="event-dashboard__header">
        <div className="event-dashboard__title-block">
          <div className="event-dashboard__icon">
            <Radio size={22} />
          </div>
          <div>
            <h2>Event Listener</h2>
            <p>Live contract event stream</p>
          </div>
        </div>

        <div className={`listener-status listener-status--${status}`} data-testid="listener-status">
          <span aria-hidden="true" />
          {status}
        </div>
      </div>

      <div className="event-dashboard__metrics" aria-label="Event listener metrics">
        <MetricTile icon={<Bell size={18} />} label="Events" value={metrics.totalEvents} />
        <MetricTile icon={<Server size={18} />} label="Contracts" value={metrics.uniqueContracts} />
        <MetricTile icon={<ShieldAlert size={18} />} label="Critical" value={metrics.criticalCount} />
        <MetricTile icon={<Clock3 size={18} />} label="Ledger" value={metrics.latestLedger.toLocaleString()} />
      </div>

      <div className="event-dashboard__toolbar">
        <label className="event-search">
          <Search size={16} />
          <input
            type="search"
            placeholder="Search events"
            value={filter.query}
            onChange={(event) => setFilter({ ...filter, query: event.target.value })}
            data-testid="event-search"
          />
        </label>

        <div className="severity-filter" aria-label="Severity filter">
          <Filter size={16} />
          {severityOptions.map((severity) => (
            <button
              key={severity}
              type="button"
              className={filter.severity === severity ? 'active' : ''}
              onClick={() => setFilter({ ...filter, severity })}
              data-testid={`severity-${severity}`}
            >
              {severityLabels[severity]}
            </button>
          ))}
        </div>

        <div className="listener-actions">
          {status === 'connected' ? (
            <button type="button" onClick={pause} data-testid="pause-feed" title="Pause live feed">
              <CirclePause size={17} />
              Pause
            </button>
          ) : (
            <button type="button" onClick={resume} data-testid="resume-feed" title="Resume live feed">
              <CirclePlay size={17} />
              Resume
            </button>
          )}
          <button type="button" onClick={disconnect} data-testid="disconnect-feed" title="Disconnect listener">
            <Unplug size={17} />
            Disconnect
          </button>
        </div>
      </div>

      <div className="event-dashboard__content">
        <div className="event-feed" data-testid="event-feed">
          <div className="event-feed__heading">
            <h3>Live Feed</h3>
            <span>{visibleEvents.length} visible</span>
          </div>

          <div className="event-feed__list">
            {visibleEvents.map((event) => (
              <EventRow
                key={event.id}
                event={event}
                active={selectedEvent?.id === event.id}
                onSelect={() => setSelectedEventId(event.id)}
              />
            ))}

            {visibleEvents.length === 0 && (
              <div className="empty-feed" data-testid="empty-feed">
                <AlertTriangle size={18} />
                No events match the current filter.
              </div>
            )}
          </div>
        </div>

        <EventDetails event={selectedEvent} />
      </div>
    </section>
  );
};

type MetricTileProps = {
  icon: React.ReactNode;
  label: string;
  value: number | string;
};

const MetricTile: React.FC<MetricTileProps> = ({ icon, label, value }) => (
  <div className="metric-tile">
    <div className="metric-tile__icon">{icon}</div>
    <div>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  </div>
);

type EventRowProps = {
  event: ContractEvent;
  active: boolean;
  onSelect: () => void;
};

const EventRow: React.FC<EventRowProps> = ({ event, active, onSelect }) => (
  <button
    type="button"
    className={`event-row ${active ? 'event-row--active' : ''}`}
    onClick={onSelect}
    data-testid={`event-row-${event.id}`}
  >
    <span className={`severity-dot severity-dot--${event.severity}`} aria-label={event.severity} />
    <span className="event-row__main">
      <span className="event-row__title">
        {event.displayName}
        <small>{event.topicLabel}</small>
      </span>
      <span className="event-row__meta">
        {shortenEventId(event.id)} / {shortenContractId(event.contractId)} / ledger{' '}
        {event.ledger.toLocaleString()}
      </span>
    </span>
    <span className="event-row__time">{formatEventTime(event.ledgerClosedAt)}</span>
  </button>
);

type EventDetailsProps = {
  event: ContractEvent | null;
};

const EventDetails: React.FC<EventDetailsProps> = ({ event }) => {
  if (!event) {
    return (
      <aside className="event-details" data-testid="event-details">
        <h3>Event Details</h3>
        <p>No event selected.</p>
      </aside>
    );
  }

  return (
    <aside className="event-details" data-testid="event-details">
      <div className="event-details__header">
        <h3>Event Details</h3>
        <span className="event-status event-status--rpc">{event.type}</span>
      </div>

      <dl>
        <div>
          <dt>Contract</dt>
          <dd>{event.contractId}</dd>
        </div>
        <div>
          <dt>Event Type</dt>
          <dd>{event.type}</dd>
        </div>
        <div>
          <dt>Display Name</dt>
          <dd>{event.displayName}</dd>
        </div>
        <div>
          <dt>Topic</dt>
          <dd>{event.topicLabel}</dd>
        </div>
        <div>
          <dt>Ledger</dt>
          <dd>{event.ledger.toLocaleString()}</dd>
        </div>
        <div>
          <dt>Ledger Closed</dt>
          <dd>{event.ledgerClosedAt}</dd>
        </div>
        <div>
          <dt>Transaction Hash</dt>
          <dd>{event.txHash}</dd>
        </div>
        <div>
          <dt>Topic XDR</dt>
          <dd>{event.topic.join(', ')}</dd>
        </div>
        <div>
          <dt>Value XDR</dt>
          <dd>{event.value}</dd>
        </div>
        <div>
          <dt>Preview</dt>
          <dd>{event.valuePreview}</dd>
        </div>
      </dl>
    </aside>
  );
};

const shortenContractId = (contractId: string): string =>
  `${contractId.slice(0, 4)}...${contractId.slice(-4)}`;

const shortenEventId = (eventId: string): string => eventId.slice(-10);
