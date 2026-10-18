import { render, screen, fireEvent, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { StorageDiffViewer } from './StorageDiffViewer';
import {
  calculateRent,
  diffStorage,
  formatBytes,
  type StorageEntry,
} from './storageDiff';

const CONTRACT = 'CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC';

const entry = (over: Partial<StorageEntry> = {}): StorageEntry => ({
  contractId: CONTRACT,
  durability: 'persistent',
  key: 'Balance(alice)',
  value: '1000',
  sizeBytes: 64,
  ...over,
});

describe('diffStorage', () => {
  it('classifies created, modified and deleted entries', () => {
    const before = [
      entry({ key: 'Balance(alice)', value: '1000' }),
      entry({ key: 'Balance(bob)', value: '500' }),
    ];
    const after = [
      entry({ key: 'Balance(alice)', value: '900' }),
      entry({ key: 'Balance(carol)', value: '100' }),
    ];

    const diff = diffStorage(before, after);
    const byKey = Object.fromEntries(diff.entries.map((e) => [e.key, e.kind]));

    expect(byKey['Balance(alice)']).toBe('modified');
    expect(byKey['Balance(bob)']).toBe('deleted');
    expect(byKey['Balance(carol)']).toBe('created');
  });

  it('omits unchanged entries by default', () => {
    const same = [entry()];
    expect(diffStorage(same, same).entries).toHaveLength(0);
  });

  it('includes unchanged entries when asked', () => {
    const same = [entry()];
    const diff = diffStorage(same, same, { includeUnchanged: true });
    expect(diff.entries).toHaveLength(1);
    expect(diff.entries[0].kind).toBe('unchanged');
  });

  it('treats the same key in different durabilities as different entries', () => {
    const before = [entry({ durability: 'persistent', value: '1' })];
    const after = [entry({ durability: 'temporary', value: '1' })];

    const diff = diffStorage(before, after);

    expect(diff.entries).toHaveLength(2);
    expect(diff.entries.map((e) => e.kind).sort()).toEqual([
      'created',
      'deleted',
    ]);
  });

  it('detects a size change even when the value preview is identical', () => {
    const before = [entry({ sizeBytes: 64 })];
    const after = [entry({ sizeBytes: 128 })];

    const diff = diffStorage(before, after);

    expect(diff.entries[0].kind).toBe('modified');
    expect(diff.entries[0].deltaBytes).toBe(64);
  });

  it('reports a negative delta when an entry shrinks', () => {
    const diff = diffStorage([entry({ sizeBytes: 128 })], [entry({ sizeBytes: 32 })]);
    expect(diff.entries[0].deltaBytes).toBe(-96);
  });

  it('orders created, then modified, then deleted', () => {
    const before = [
      entry({ key: 'zzz-deleted' }),
      entry({ key: 'mmm-modified', value: 'old' }),
    ];
    const after = [
      entry({ key: 'mmm-modified', value: 'new' }),
      entry({ key: 'aaa-created' }),
    ];

    expect(diffStorage(before, after).entries.map((e) => e.kind)).toEqual([
      'created',
      'modified',
      'deleted',
    ]);
  });

  it('summarises per durability and totals the footprint', () => {
    const before = [entry({ key: 'k1', durability: 'persistent', sizeBytes: 100 })];
    const after = [
      entry({ key: 'k1', durability: 'persistent', sizeBytes: 150, value: 'v2' }),
      entry({ key: 'k2', durability: 'temporary', sizeBytes: 40 }),
    ];

    const diff = diffStorage(before, after);
    const persistent = diff.summary.find((s) => s.durability === 'persistent')!;
    const temporary = diff.summary.find((s) => s.durability === 'temporary')!;

    expect(persistent.modified).toBe(1);
    expect(persistent.deltaBytes).toBe(50);
    expect(temporary.created).toBe(1);
    expect(diff.totalDeltaBytes).toBe(90);
  });
});

describe('calculateRent', () => {
  it('charges temporary storage less than persistent for the same bytes', () => {
    expect(calculateRent(1000, 'temporary')).toBeLessThan(
      calculateRent(1000, 'persistent'),
    );
  });

  it('charges nothing for a deleted entry rather than crediting rent', () => {
    // The network does not refund rent already paid, so a credit would
    // misrepresent what the transaction costs.
    expect(calculateRent(0, 'persistent')).toBe(0);
  });

  it('scales with the projected TTL', () => {
    expect(calculateRent(100, 'persistent', 2000)).toBe(
      calculateRent(100, 'persistent', 1000) * 2,
    );
  });
});

describe('formatBytes', () => {
  it('signs a positive delta only when asked', () => {
    expect(formatBytes(64, true)).toBe('+64 B');
    expect(formatBytes(64)).toBe('64 B');
    expect(formatBytes(-64, true)).toBe('-64 B');
  });

  it('switches to KiB past a kilobyte', () => {
    expect(formatBytes(2048)).toBe('2.00 KiB');
  });
});

describe('StorageDiffViewer', () => {
  const before = [
    entry({ key: 'Balance(alice)', value: '1000', sizeBytes: 64 }),
    entry({ key: 'Balance(bob)', value: '500', sizeBytes: 64 }),
  ];
  const after = [
    entry({ key: 'Balance(alice)', value: '900', sizeBytes: 64 }),
    entry({ key: 'Nonce', value: '1', sizeBytes: 16, durability: 'temporary' }),
  ];

  it('renders a row per changed entry', () => {
    render(<StorageDiffViewer before={before} after={after} />);

    expect(screen.getByTestId('storage-diff-viewer')).toBeInTheDocument();
    expect(screen.getByText('Balance(alice)')).toBeInTheDocument();
    expect(screen.getByText('Balance(bob)')).toBeInTheDocument();
    expect(screen.getByText('Nonce')).toBeInTheDocument();
  });

  it('labels every row, so colour is never the only signal', () => {
    render(<StorageDiffViewer before={before} after={after} />);

    expect(screen.getByText('Created')).toBeInTheDocument();
    expect(screen.getByText('Modified')).toBeInTheDocument();
    expect(screen.getByText('Deleted')).toBeInTheDocument();
  });

  it('hides entry values until a row is expanded', () => {
    render(<StorageDiffViewer before={before} after={after} />);

    expect(
      screen.queryByTestId('detail-persistent-Balance(alice)'),
    ).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId('toggle-persistent-Balance(alice)'));

    const detail = screen.getByTestId('detail-persistent-Balance(alice)');
    expect(within(detail).getByText('1000')).toBeInTheDocument();
    expect(within(detail).getByText('900')).toBeInTheDocument();
  });

  it('filters by durability when a summary card is clicked', () => {
    render(<StorageDiffViewer before={before} after={after} />);

    fireEvent.click(screen.getByTestId('summary-temporary'));

    expect(screen.getByText('Nonce')).toBeInTheDocument();
    expect(screen.queryByText('Balance(alice)')).not.toBeInTheDocument();
  });

  it('clears the filter when the active card is clicked again', () => {
    render(<StorageDiffViewer before={before} after={after} />);

    fireEvent.click(screen.getByTestId('summary-temporary'));
    fireEvent.click(screen.getByTestId('summary-temporary'));

    expect(screen.getByText('Balance(alice)')).toBeInTheDocument();
  });

  it('says so plainly when nothing changed', () => {
    render(<StorageDiffViewer before={before} after={before} />);
    expect(screen.getByTestId('storage-diff-empty')).toBeInTheDocument();
  });

  it('shows the total footprint delta', () => {
    render(<StorageDiffViewer before={before} after={after} />);

    const totals = screen.getByTestId('storage-diff-totals');
    // -64 B for the deleted balance, +16 B for the new temporary nonce.
    expect(within(totals).getByText('-48 B')).toBeInTheDocument();
  });
});
