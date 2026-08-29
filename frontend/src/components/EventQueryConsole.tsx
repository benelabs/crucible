import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Download, Filter, Pause, Play, Search, Trash2 } from 'lucide-react';
import './EventQueryConsole.css';

/* ── Stream model ─────────────────────────────────────────────────────────── */

export interface StreamEvent {
  id: string;
  ledger: number;
  timestamp: string;
  contractId: string;
  topic: string;
  type: 'transfer' | 'mint' | 'burn' | 'approve' | 'error';
  amount: number;
  from: string;
  to: string;
}

/** Rows are capped so an unbounded stream cannot grow the buffer forever. */
export const MAX_BUFFERED_EVENTS = 500;

/* ── JSONPath subset ──────────────────────────────────────────────────────── */

export type ComparisonOperator = '==' | '!=' | '>' | '>=' | '<' | '<=' | '=~';

export type JsonPathSegment =
  | { kind: 'child'; name: string }
  | { kind: 'index'; index: number }
  | { kind: 'wildcard' }
  | { kind: 'descend'; name: string }
  | { kind: 'filter'; field: string[]; operator?: ComparisonOperator; value?: unknown };

const FILTER_BODY = /^\?\(\s*@\.([A-Za-z0-9_.]+)\s*(?:(==|!=|>=|<=|>|<|=~)\s*(.+?))?\s*\)$/;

function parseLiteral(raw: string): unknown {
  const value = raw.trim();
  if (value === 'true') return true;
  if (value === 'false') return false;
  if (value === 'null') return null;
  if (/^-?\d+(\.\d+)?$/.test(value)) return Number(value);
  if (/^'.*'$/.test(value) || /^".*"$/.test(value)) return value.slice(1, -1);
  if (/^\/.*\/$/.test(value)) return value.slice(1, -1);
  return value;
}

/**
 * Parses the supported JSONPath subset:
 *   `$`, `.name`, `['name']`, `[n]`, `[*]`, `.*`, `..name`
 *   and filters `[?(@.field)]` / `[?(@.field <op> value)]`
 *   where <op> is == != > >= < <= or =~ (regular expression).
 * Throws on anything outside that grammar rather than silently matching all.
 */
export function parseJsonPath(expression: string): JsonPathSegment[] {
  const input = expression.trim();
  if (input === '') throw new Error('Query is empty');
  if (input[0] !== '$') throw new Error('Query must start with $');

  const segments: JsonPathSegment[] = [];
  let cursor = 1;

  while (cursor < input.length) {
    const character = input[cursor];

    if (character === '.') {
      if (input[cursor + 1] === '.') {
        const match = /^[A-Za-z0-9_]+/.exec(input.slice(cursor + 2));
        if (!match) throw new Error('Expected a property name after ".."');
        segments.push({ kind: 'descend', name: match[0] });
        cursor += 2 + match[0].length;
        continue;
      }
      if (input[cursor + 1] === '*') {
        segments.push({ kind: 'wildcard' });
        cursor += 2;
        continue;
      }
      const match = /^[A-Za-z0-9_]+/.exec(input.slice(cursor + 1));
      if (!match) throw new Error('Expected a property name after "."');
      segments.push({ kind: 'child', name: match[0] });
      cursor += 1 + match[0].length;
      continue;
    }

    if (character === '[') {
      const close = input.indexOf(']', cursor);
      if (close === -1) throw new Error('Unclosed "[" in query');
      const body = input.slice(cursor + 1, close).trim();
      cursor = close + 1;

      if (body === '*') {
        segments.push({ kind: 'wildcard' });
        continue;
      }
      if (/^-?\d+$/.test(body)) {
        segments.push({ kind: 'index', index: Number(body) });
        continue;
      }
      if (/^'.*'$/.test(body) || /^".*"$/.test(body)) {
        segments.push({ kind: 'child', name: body.slice(1, -1) });
        continue;
      }
      const filter = FILTER_BODY.exec(body);
      if (filter) {
        segments.push({
          kind: 'filter',
          field: filter[1].split('.'),
          operator: filter[2] as ComparisonOperator | undefined,
          value: filter[3] === undefined ? undefined : parseLiteral(filter[3]),
        });
        continue;
      }
      throw new Error(`Unsupported selector: [${body}]`);
    }

    throw new Error(`Unexpected character "${character}" at position ${cursor}`);
  }

  return segments;
}

function readPath(node: unknown, path: string[]): unknown {
  return path.reduce<unknown>((current, key) => {
    if (current === null || typeof current !== 'object') return undefined;
    return (current as Record<string, unknown>)[key];
  }, node);
}

export function compareValues(left: unknown, operator: ComparisonOperator, right: unknown): boolean {
  if (operator === '=~') {
    try {
      return new RegExp(String(right)).test(String(left));
    } catch {
      return false;
    }
  }
  if (operator === '==') return String(left) === String(right);
  if (operator === '!=') return String(left) !== String(right);

  const a = Number(left);
  const b = Number(right);
  if (Number.isNaN(a) || Number.isNaN(b)) return false;

  if (operator === '>') return a > b;
  if (operator === '>=') return a >= b;
  if (operator === '<') return a < b;
  return a <= b;
}

const childValues = (node: unknown): unknown[] => {
  if (Array.isArray(node)) return node;
  if (node !== null && typeof node === 'object') return Object.values(node as Record<string, unknown>);
  return [];
};

function descend(node: unknown, name: string, found: unknown[]): void {
  if (node === null || typeof node !== 'object') return;
  if (!Array.isArray(node) && name in (node as Record<string, unknown>)) {
    found.push((node as Record<string, unknown>)[name]);
  }
  for (const child of childValues(node)) {
    descend(child, name, found);
  }
}

/** Evaluates a parsed query, returning every matching node. */
export function evaluateJsonPath(expression: string, root: unknown): unknown[] {
  const segments = parseJsonPath(expression);
  let current: unknown[] = [root];

  for (const segment of segments) {
    const next: unknown[] = [];

    for (const node of current) {
      if (segment.kind === 'child') {
        if (node !== null && typeof node === 'object' && !Array.isArray(node)) {
          const value = (node as Record<string, unknown>)[segment.name];
          if (value !== undefined) next.push(value);
        }
      } else if (segment.kind === 'index') {
        if (Array.isArray(node)) {
          const index = segment.index < 0 ? node.length + segment.index : segment.index;
          if (index >= 0 && index < node.length) next.push(node[index]);
        }
      } else if (segment.kind === 'wildcard') {
        next.push(...childValues(node));
      } else if (segment.kind === 'descend') {
        descend(node, segment.name, next);
      } else {
        for (const candidate of childValues(node)) {
          const actual = readPath(candidate, segment.field);
          const matches =
            segment.operator === undefined
              ? actual !== undefined && actual !== null && actual !== false
              : compareValues(actual, segment.operator, segment.value);
          if (matches) next.push(candidate);
        }
      }
    }

    current = next;
  }

  return current;
}

export interface QueryOutcome {
  events: StreamEvent[];
  matches: unknown[];
  error?: string;
}

/**
 * Runs a query against the stream envelope `{ data: events }`, so the
 * documented `$.data[?(@.amount > 1000)]` form selects events directly.
 * Events are matched by identity, so scalar projections select no rows.
 */
export function queryEvents(events: StreamEvent[], expression: string): QueryOutcome {
  if (expression.trim() === '') {
    return { events, matches: events };
  }

  try {
    const matches = evaluateJsonPath(expression, { data: events });
    const matched = new Set(matches);
    return { events: events.filter((event) => matched.has(event)), matches };
  } catch (error) {
    return {
      events: [],
      matches: [],
      error: error instanceof Error ? error.message : 'Invalid query',
    };
  }
}

/** Filters by a regular expression over the event topic. */
export function filterByTopicRegex(events: StreamEvent[], pattern: string): { events: StreamEvent[]; error?: string } {
  if (pattern.trim() === '') return { events };
  try {
    const regex = new RegExp(pattern);
    return { events: events.filter((event) => regex.test(event.topic)) };
  } catch (error) {
    return { events: [], error: error instanceof Error ? error.message : 'Invalid regular expression' };
  }
}

/* ── Export ───────────────────────────────────────────────────────────────── */

export const CSV_COLUMNS: (keyof StreamEvent)[] = [
  'id',
  'ledger',
  'timestamp',
  'contractId',
  'topic',
  'type',
  'amount',
  'from',
  'to',
];

/** Quotes a CSV field only when it contains a delimiter, quote or newline. */
export function escapeCsv(value: unknown): string {
  const text = value === null || value === undefined ? '' : String(value);
  return /[",\n\r]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text;
}

export function toCsv(events: StreamEvent[]): string {
  return [
    CSV_COLUMNS.join(','),
    ...events.map((event) => CSV_COLUMNS.map((column) => escapeCsv(event[column])).join(',')),
  ].join('\n');
}

export function toJson(events: StreamEvent[]): string {
  return JSON.stringify(events, null, 2);
}

export function downloadText(text: string, fileName: string, mimeType: string): void {
  const url = URL.createObjectURL(new Blob([text], { type: mimeType }));
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = fileName;
  document.body.appendChild(anchor);
  anchor.click();
  document.body.removeChild(anchor);
  URL.revokeObjectURL(url);
}

/* ── Live stream ──────────────────────────────────────────────────────────── */

export interface EventSocket {
  close: () => void;
}

export type EventSocketFactory = (onEvent: (event: StreamEvent) => void) => EventSocket;

export const DEFAULT_STREAM_URL = 'ws://localhost:3000/api/v1/events/stream';

/** Real WebSocket transport; each frame is one JSON-encoded StreamEvent. */
export const createWebSocketStream =
  (url: string = DEFAULT_STREAM_URL): EventSocketFactory =>
  (onEvent) => {
    const socket = new WebSocket(url);
    socket.onmessage = (message) => {
      try {
        onEvent(JSON.parse(message.data as string) as StreamEvent);
      } catch {
        // A malformed frame must not tear down the stream.
      }
    };
    return { close: () => socket.close() };
  };

export const SAMPLE_EVENTS: StreamEvent[] = [
  {
    id: 'evt-1',
    ledger: 12940250,
    timestamp: '2026-08-29T11:58:04.000Z',
    contractId: 'CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC',
    topic: 'token.transfer',
    type: 'transfer',
    amount: 12500,
    from: 'GTREASURY',
    to: 'GVAULT',
  },
  {
    id: 'evt-2',
    ledger: 12940251,
    timestamp: '2026-08-29T11:58:09.000Z',
    contractId: 'CBQX2CLT7JFPASGQYQ6B6HR5IE23DVKSWJEVFXT7Y7AKLZ4E5YGH71MD',
    topic: 'allowance.approved',
    type: 'approve',
    amount: 250,
    from: 'GMARKET',
    to: 'GROUTER',
  },
  {
    id: 'evt-3',
    ledger: 12940252,
    timestamp: '2026-08-29T11:58:14.000Z',
    contractId: 'CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC',
    topic: 'token.mint',
    type: 'mint',
    amount: 90000,
    from: 'GISSUER',
    to: 'GTREASURY',
  },
  {
    id: 'evt-4',
    ledger: 12940253,
    timestamp: '2026-08-29T11:58:19.000Z',
    contractId: 'CC4RQ3KX37R4XTQGDN3Q6O5IPTVRUKZSDHFDMB4JYCNKUEK9JH6B20KV',
    topic: 'escrow.error',
    type: 'error',
    amount: 0,
    from: 'GESCROW',
    to: 'GBUYER',
  },
];

export interface EventQueryConsoleProps {
  initialEvents?: StreamEvent[];
  socketFactory?: EventSocketFactory;
}

export const EventQueryConsole: React.FC<EventQueryConsoleProps> = ({
  initialEvents = SAMPLE_EVENTS,
  socketFactory,
}) => {
  const [events, setEvents] = useState<StreamEvent[]>(initialEvents);
  const [query, setQuery] = useState('');
  const [topicPattern, setTopicPattern] = useState('');
  const [live, setLive] = useState(true);
  const liveRef = useRef(live);
  useEffect(() => {
    liveRef.current = live;
  });

  // Frames arriving while paused are dropped, so the table stays inspectable.
  useEffect(() => {
    if (!socketFactory) return undefined;
    const socket = socketFactory((event) => {
      if (!liveRef.current) return;
      setEvents((previous) => [...previous, event].slice(-MAX_BUFFERED_EVENTS));
    });
    return () => socket.close();
  }, [socketFactory]);

  const topicFiltered = useMemo(() => filterByTopicRegex(events, topicPattern), [events, topicPattern]);
  const outcome = useMemo(() => queryEvents(topicFiltered.events, query), [topicFiltered.events, query]);

  const visible = outcome.events;
  const error = topicFiltered.error ?? outcome.error;

  const handleExport = useCallback(
    (format: 'json' | 'csv') => {
      if (format === 'json') {
        downloadText(toJson(visible), 'crucible-events.json', 'application/json');
      } else {
        downloadText(toCsv(visible), 'crucible-events.csv', 'text/csv');
      }
    },
    [visible],
  );

  return (
    <div className="event-console-container" data-testid="event-query-console">
      <div className="event-console-header">
        <div className="event-console-icon-wrapper">
          <Search className="event-console-icon" />
        </div>
        <div>
          <h2>Event Query Console</h2>
          <p>JSONPath querying and topic regex filtering over the live event stream</p>
        </div>
      </div>

      <div className="event-console-controls">
        <label className="event-console-field">
          <span className="event-console-label">JSONPath query</span>
          <input
            className="event-console-input"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="$.data[?(@.amount > 1000)]"
            data-testid="query-input"
          />
        </label>

        <label className="event-console-field">
          <span className="event-console-label">Topic regex</span>
          <input
            className="event-console-input"
            value={topicPattern}
            onChange={(event) => setTopicPattern(event.target.value)}
            placeholder="^token\\."
            data-testid="topic-input"
          />
        </label>

        <div className="event-console-buttons">
          <button
            type="button"
            className={`event-console-btn ${live ? 'active' : ''}`}
            onClick={() => setLive((value) => !value)}
            aria-pressed={live}
            data-testid="toggle-live"
          >
            {live ? <Pause size={14} /> : <Play size={14} />}
            {live ? 'Pause' : 'Resume'}
          </button>
          <button
            type="button"
            className="event-console-btn"
            onClick={() => handleExport('json')}
            data-testid="export-json"
          >
            <Download size={14} />
            JSON
          </button>
          <button
            type="button"
            className="event-console-btn"
            onClick={() => handleExport('csv')}
            data-testid="export-csv"
          >
            <Download size={14} />
            CSV
          </button>
          <button
            type="button"
            className="event-console-btn"
            onClick={() => setEvents([])}
            aria-label="Clear buffer"
            data-testid="clear-buffer"
          >
            <Trash2 size={14} />
          </button>
        </div>
      </div>

      <div className="event-console-status">
        <span data-testid="stream-status" data-live={String(live)}>
          {live ? 'Live' : 'Paused'}
        </span>
        <span data-testid="result-count">
          <Filter size={12} />
          {visible.length} of {events.length} events
        </span>
      </div>

      {error && (
        <p className="event-console-error" data-testid="query-error">
          {error}
        </p>
      )}

      <div className="event-console-table-wrapper">
        <table className="event-console-table">
          <thead>
            <tr>
              <th>Ledger</th>
              <th>Topic</th>
              <th>Type</th>
              <th>Amount</th>
              <th>From</th>
              <th>To</th>
            </tr>
          </thead>
          <tbody data-testid="event-rows">
            {visible.map((event) => (
              <tr key={event.id} data-testid={`event-${event.id}`}>
                <td>{event.ledger}</td>
                <td>{event.topic}</td>
                <td>{event.type}</td>
                <td>{event.amount.toLocaleString('en-US')}</td>
                <td>{event.from}</td>
                <td>{event.to}</td>
              </tr>
            ))}
          </tbody>
        </table>
        {visible.length === 0 && !error && (
          <p className="event-console-empty" data-testid="no-results">
            No events match the current filters.
          </p>
        )}
      </div>
    </div>
  );
};
