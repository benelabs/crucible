import { render, screen, fireEvent, act } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  EventQueryConsole,
  MAX_BUFFERED_EVENTS,
  SAMPLE_EVENTS,
  compareValues,
  escapeCsv,
  evaluateJsonPath,
  filterByTopicRegex,
  parseJsonPath,
  queryEvents,
  toCsv,
  toJson,
  type EventSocketFactory,
  type StreamEvent,
} from './EventQueryConsole';

const event = (overrides: Partial<StreamEvent> = {}): StreamEvent => ({
  id: 'evt-x',
  ledger: 1,
  timestamp: '2026-08-29T12:00:00.000Z',
  contractId: 'CDLZ',
  topic: 'token.transfer',
  type: 'transfer',
  amount: 100,
  from: 'GA',
  to: 'GB',
  ...overrides,
});

describe('parseJsonPath', () => {
  it('parses child, index and wildcard selectors', () => {
    expect(parseJsonPath('$.data[0].amount')).toEqual([
      { kind: 'child', name: 'data' },
      { kind: 'index', index: 0 },
      { kind: 'child', name: 'amount' },
    ]);
    expect(parseJsonPath('$.data[*]')).toEqual([
      { kind: 'child', name: 'data' },
      { kind: 'wildcard' },
    ]);
    expect(parseJsonPath('$.data.*')).toEqual([
      { kind: 'child', name: 'data' },
      { kind: 'wildcard' },
    ]);
  });

  it('parses bracketed and recursive property access', () => {
    expect(parseJsonPath("$['data']")).toEqual([{ kind: 'child', name: 'data' }]);
    expect(parseJsonPath('$..amount')).toEqual([{ kind: 'descend', name: 'amount' }]);
  });

  it('parses a comparison filter', () => {
    expect(parseJsonPath('$.data[?(@.amount > 1000)]')).toEqual([
      { kind: 'child', name: 'data' },
      { kind: 'filter', field: ['amount'], operator: '>', value: 1000 },
    ]);
  });

  it('parses filters over nested fields and quoted literals', () => {
    expect(parseJsonPath("$.data[?(@.meta.kind == 'mint')]")).toEqual([
      { kind: 'child', name: 'data' },
      { kind: 'filter', field: ['meta', 'kind'], operator: '==', value: 'mint' },
    ]);
  });

  it('parses an existence filter', () => {
    expect(parseJsonPath('$.data[?(@.amount)]')).toEqual([
      { kind: 'child', name: 'data' },
      { kind: 'filter', field: ['amount'], operator: undefined, value: undefined },
    ]);
  });

  it('rejects malformed queries rather than matching everything', () => {
    expect(() => parseJsonPath('')).toThrow('Query is empty');
    expect(() => parseJsonPath('data.amount')).toThrow('Query must start with $');
    expect(() => parseJsonPath('$.data[0')).toThrow('Unclosed "[" in query');
    expect(() => parseJsonPath('$.')).toThrow('Expected a property name after "."');
    expect(() => parseJsonPath('$.data[?(bogus)]')).toThrow('Unsupported selector');
    expect(() => parseJsonPath('$#data')).toThrow('Unexpected character');
  });
});

describe('compareValues', () => {
  it('compares numerically for ordering operators', () => {
    expect(compareValues(2000, '>', 1000)).toBe(true);
    expect(compareValues('2000', '>=', 2000)).toBe(true);
    expect(compareValues(500, '<', 1000)).toBe(true);
    expect(compareValues(1000, '<=', 1000)).toBe(true);
  });

  it('is false when either side is not numeric', () => {
    expect(compareValues('abc', '>', 1000)).toBe(false);
    expect(compareValues(undefined, '>', 1000)).toBe(false);
  });

  it('compares equality as strings so 1 and "1" agree', () => {
    expect(compareValues(1, '==', '1')).toBe(true);
    expect(compareValues('mint', '!=', 'burn')).toBe(true);
  });

  it('applies regular expressions for =~', () => {
    expect(compareValues('token.transfer', '=~', '^token\\.')).toBe(true);
    expect(compareValues('escrow.error', '=~', '^token\\.')).toBe(false);
    // An invalid pattern matches nothing instead of throwing.
    expect(compareValues('anything', '=~', '([')).toBe(false);
  });
});

describe('evaluateJsonPath', () => {
  const root = {
    data: [
      { amount: 100, meta: { kind: 'transfer' } },
      { amount: 5000, meta: { kind: 'mint' } },
    ],
  };

  it('reads a nested value', () => {
    expect(evaluateJsonPath('$.data[1].amount', root)).toEqual([5000]);
  });

  it('expands a wildcard over an array', () => {
    expect(evaluateJsonPath('$.data[*].amount', root)).toEqual([100, 5000]);
  });

  it('applies a comparison filter', () => {
    expect(evaluateJsonPath('$.data[?(@.amount > 1000)]', root)).toEqual([root.data[1]]);
  });

  it('filters on a nested field', () => {
    expect(evaluateJsonPath("$.data[?(@.meta.kind == 'transfer')]", root)).toEqual([root.data[0]]);
  });

  it('collects values recursively', () => {
    expect(evaluateJsonPath('$..amount', root)).toEqual([100, 5000]);
  });

  it('supports negative indexing', () => {
    expect(evaluateJsonPath('$.data[-1].amount', root)).toEqual([5000]);
  });

  it('returns nothing for a path that does not exist', () => {
    expect(evaluateJsonPath('$.missing.deeper', root)).toEqual([]);
    expect(evaluateJsonPath('$.data[9]', root)).toEqual([]);
  });
});

describe('queryEvents', () => {
  const events = [
    event({ id: 'a', amount: 100, type: 'transfer' }),
    event({ id: 'b', amount: 12500, type: 'mint', topic: 'token.mint' }),
    event({ id: 'c', amount: 0, type: 'error', topic: 'escrow.error' }),
  ];

  it('returns everything for an empty query', () => {
    expect(queryEvents(events, '   ').events).toEqual(events);
  });

  it('runs the documented amount filter', () => {
    const outcome = queryEvents(events, '$.data[?(@.amount > 1000)]');

    expect(outcome.events.map((item) => item.id)).toEqual(['b']);
    expect(outcome.error).toBeUndefined();
  });

  it('filters by an equality match on a string field', () => {
    expect(queryEvents(events, "$.data[?(@.type == 'error')]").events.map((item) => item.id)).toEqual(['c']);
  });

  it('filters by regular expression over a field', () => {
    expect(
      queryEvents(events, "$.data[?(@.topic =~ '^token\\.')]").events.map((item) => item.id),
    ).toEqual(['a', 'b']);
  });

  it('selects every event with a wildcard', () => {
    expect(queryEvents(events, '$.data[*]').events).toHaveLength(3);
  });

  it('matches no rows when the query projects scalars', () => {
    const outcome = queryEvents(events, '$.data[*].amount');

    expect(outcome.events).toEqual([]);
    expect(outcome.matches).toEqual([100, 12500, 0]);
  });

  it('reports a query error instead of throwing', () => {
    const outcome = queryEvents(events, '$.data[?(nope)]');

    expect(outcome.events).toEqual([]);
    expect(outcome.error).toContain('Unsupported selector');
  });
});

describe('filterByTopicRegex', () => {
  const events = [event({ id: 'a', topic: 'token.transfer' }), event({ id: 'b', topic: 'escrow.error' })];

  it('returns everything for an empty pattern', () => {
    expect(filterByTopicRegex(events, '').events).toEqual(events);
  });

  it('keeps only matching topics', () => {
    expect(filterByTopicRegex(events, '^token\\.').events.map((item) => item.id)).toEqual(['a']);
  });

  it('reports an invalid regular expression', () => {
    const result = filterByTopicRegex(events, '([');

    expect(result.events).toEqual([]);
    expect(result.error).toBeTruthy();
  });
});

describe('escapeCsv', () => {
  it('leaves plain values alone', () => {
    expect(escapeCsv('token.transfer')).toBe('token.transfer');
    expect(escapeCsv(1200)).toBe('1200');
  });

  it('quotes and doubles embedded quotes, commas and newlines', () => {
    expect(escapeCsv('a,b')).toBe('"a,b"');
    expect(escapeCsv('say "hi"')).toBe('"say ""hi"""');
    expect(escapeCsv('line\nbreak')).toBe('"line\nbreak"');
  });

  it('renders null and undefined as empty', () => {
    expect(escapeCsv(null)).toBe('');
    expect(escapeCsv(undefined)).toBe('');
  });
});

describe('toCsv / toJson', () => {
  it('writes a header row followed by one row per event', () => {
    const csv = toCsv([event({ id: 'a', amount: 100 })]).split('\n');

    expect(csv[0]).toBe('id,ledger,timestamp,contractId,topic,type,amount,from,to');
    expect(csv[1]).toBe('a,1,2026-08-29T12:00:00.000Z,CDLZ,token.transfer,transfer,100,GA,GB');
  });

  it('writes only the header for an empty selection', () => {
    expect(toCsv([]).split('\n')).toHaveLength(1);
  });

  it('serialises events as indented JSON', () => {
    expect(JSON.parse(toJson([event({ id: 'a' })]))).toEqual([event({ id: 'a' })]);
  });
});

describe('EventQueryConsole', () => {
  beforeEach(() => {
    URL.createObjectURL = vi.fn(() => 'blob:events');
    URL.revokeObjectURL = vi.fn();
  });

  it('renders every sample event by default', () => {
    render(<EventQueryConsole />);

    expect(screen.getByTestId('result-count')).toHaveTextContent(
      `${SAMPLE_EVENTS.length} of ${SAMPLE_EVENTS.length} events`,
    );
  });

  it('narrows the table with a JSONPath query', () => {
    render(<EventQueryConsole />);

    fireEvent.change(screen.getByTestId('query-input'), {
      target: { value: '$.data[?(@.amount > 1000)]' },
    });

    expect(screen.getByTestId('event-evt-1')).toBeInTheDocument();
    expect(screen.getByTestId('event-evt-3')).toBeInTheDocument();
    expect(screen.queryByTestId('event-evt-2')).not.toBeInTheDocument();
    expect(screen.getByTestId('result-count')).toHaveTextContent('2 of 4 events');
  });

  it('narrows the table with a topic regex', () => {
    render(<EventQueryConsole />);

    fireEvent.change(screen.getByTestId('topic-input'), { target: { value: '^token\\.' } });

    expect(screen.getByTestId('result-count')).toHaveTextContent('2 of 4 events');
  });

  it('combines the topic regex with the JSONPath query', () => {
    render(<EventQueryConsole />);

    fireEvent.change(screen.getByTestId('topic-input'), { target: { value: '^token\\.' } });
    fireEvent.change(screen.getByTestId('query-input'), {
      target: { value: '$.data[?(@.amount > 50000)]' },
    });

    expect(screen.getByTestId('result-count')).toHaveTextContent('1 of 4 events');
    expect(screen.getByTestId('event-evt-3')).toBeInTheDocument();
  });

  it('surfaces an invalid query without clearing the buffer', () => {
    render(<EventQueryConsole />);

    fireEvent.change(screen.getByTestId('query-input'), { target: { value: 'data.amount' } });

    expect(screen.getByTestId('query-error')).toHaveTextContent('Query must start with $');
    expect(screen.getByTestId('result-count')).toHaveTextContent('0 of 4 events');
  });

  it('shows an empty state when nothing matches', () => {
    render(<EventQueryConsole />);

    fireEvent.change(screen.getByTestId('query-input'), {
      target: { value: '$.data[?(@.amount > 999999)]' },
    });

    expect(screen.getByTestId('no-results')).toBeInTheDocument();
  });

  it('appends live events and stops while paused', () => {
    let emit: ((event: StreamEvent) => void) | null = null;
    const close = vi.fn();
    const socketFactory: EventSocketFactory = (onEvent) => {
      emit = onEvent;
      return { close };
    };

    render(<EventQueryConsole initialEvents={[]} socketFactory={socketFactory} />);

    act(() => emit!(event({ id: 'live-1' })));
    expect(screen.getByTestId('event-live-1')).toBeInTheDocument();

    fireEvent.click(screen.getByTestId('toggle-live'));
    expect(screen.getByTestId('stream-status')).toHaveTextContent('Paused');

    act(() => emit!(event({ id: 'live-2' })));
    expect(screen.queryByTestId('event-live-2')).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId('toggle-live'));
    act(() => emit!(event({ id: 'live-3' })));
    expect(screen.getByTestId('event-live-3')).toBeInTheDocument();
  });

  it('caps the buffer at the retention limit', () => {
    let emit: ((event: StreamEvent) => void) | null = null;
    const socketFactory: EventSocketFactory = (onEvent) => {
      emit = onEvent;
      return { close: vi.fn() };
    };

    render(<EventQueryConsole initialEvents={[]} socketFactory={socketFactory} />);

    act(() => {
      for (let index = 0; index < MAX_BUFFERED_EVENTS + 10; index += 1) {
        emit!(event({ id: `live-${index}` }));
      }
    });

    expect(screen.getByTestId('result-count')).toHaveTextContent(
      `${MAX_BUFFERED_EVENTS} of ${MAX_BUFFERED_EVENTS} events`,
    );
  });

  it('closes the socket on unmount', () => {
    const close = vi.fn();
    const { unmount } = render(
      <EventQueryConsole initialEvents={[]} socketFactory={() => ({ close })} />,
    );

    unmount();

    expect(close).toHaveBeenCalled();
  });

  it('clears the buffer on demand', () => {
    render(<EventQueryConsole />);

    fireEvent.click(screen.getByTestId('clear-buffer'));

    expect(screen.getByTestId('result-count')).toHaveTextContent('0 of 0 events');
  });

  it('exports the filtered selection as JSON and CSV', () => {
    render(<EventQueryConsole />);

    fireEvent.click(screen.getByTestId('export-json'));
    fireEvent.click(screen.getByTestId('export-csv'));

    expect(URL.createObjectURL).toHaveBeenCalledTimes(2);
  });
});
