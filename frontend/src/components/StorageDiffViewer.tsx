import React, { useMemo, useState } from 'react';
import {
  Database,
  ChevronRight,
  ChevronDown,
  PlusCircle,
  MinusCircle,
  PencilLine,
} from 'lucide-react';

import './StorageDiffViewer.css';
import {
  DEFAULT_TTL_LEDGERS,
  diffStorage,
  formatBytes,
  formatStroops,
  type ChangeKind,
  type Durability,
  type StorageDiffEntry,
  type StorageEntry,
} from './storageDiff';

export interface StorageDiffViewerProps {
  /** Footprint before simulation. */
  before: StorageEntry[];
  /** Footprint after simulation. */
  after: StorageEntry[];
  /** Ledgers to project rent over. */
  ttlLedgers?: number;
}

const DURABILITY_LABEL: Record<Durability, string> = {
  instance: 'Instance',
  persistent: 'Persistent',
  temporary: 'Temporary',
};

const KIND_LABEL: Record<ChangeKind, string> = {
  created: 'Created',
  modified: 'Modified',
  deleted: 'Deleted',
  unchanged: 'Unchanged',
};

function KindIcon({ kind }: { kind: ChangeKind }) {
  if (kind === 'created') return <PlusCircle size={14} aria-hidden="true" />;
  if (kind === 'deleted') return <MinusCircle size={14} aria-hidden="true" />;
  if (kind === 'modified') return <PencilLine size={14} aria-hidden="true" />;
  return null;
}

/**
 * Row for one changed entry. Collapsed it shows the key, size and rent;
 * expanded it shows the before and after values.
 *
 * Values are only rendered when a row is expanded — a footprint can carry
 * long serialised values, and rendering every one of them up front is what
 * makes a raw dump unusable.
 */
function DiffRow({ entry }: { entry: StorageDiffEntry }) {
  const [open, setOpen] = useState(false);
  const rowId = `${entry.durability}-${entry.key}`;

  return (
    <>
      <tr
        className={`diff-row diff-row--${entry.kind}`}
        data-testid={`diff-row-${rowId}`}
      >
        <td>
          <button
            type="button"
            className="diff-row__toggle"
            aria-expanded={open}
            aria-label={`${open ? 'Collapse' : 'Expand'} ${entry.key}`}
            data-testid={`toggle-${rowId}`}
            onClick={() => setOpen((v) => !v)}
          >
            {open ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
          </button>
        </td>
        <td>
          <span className={`diff-badge diff-badge--${entry.kind}`}>
            <KindIcon kind={entry.kind} />
            {KIND_LABEL[entry.kind]}
          </span>
        </td>
        <td className="diff-row__key">{entry.key}</td>
        <td>{DURABILITY_LABEL[entry.durability]}</td>
        <td className="diff-row__numeric">{formatBytes(entry.afterBytes)}</td>
        <td
          className={`diff-row__numeric ${
            entry.deltaBytes > 0
              ? 'is-growth'
              : entry.deltaBytes < 0
                ? 'is-shrink'
                : ''
          }`}
        >
          {formatBytes(entry.deltaBytes, true)}
        </td>
        <td className="diff-row__numeric">{formatStroops(entry.rentStroops)}</td>
      </tr>
      {open && (
        <tr className="diff-row__detail" data-testid={`detail-${rowId}`}>
          <td colSpan={7}>
            <div className="diff-values">
              <div className="diff-values__side">
                <span className="diff-values__label">Before</span>
                <code>{entry.before ?? '—'}</code>
              </div>
              <div className="diff-values__side">
                <span className="diff-values__label">After</span>
                <code>{entry.after ?? '—'}</code>
              </div>
            </div>
          </td>
        </tr>
      )}
    </>
  );
}

/**
 * Real-time storage diff and footprint visualiser (issue #889).
 *
 * Shows what a simulated transaction did to contract state: which entries it
 * created, modified and deleted, how many bytes each change costs, and the
 * rent that follows from it.
 */
export const StorageDiffViewer: React.FC<StorageDiffViewerProps> = ({
  before,
  after,
  ttlLedgers = DEFAULT_TTL_LEDGERS,
}) => {
  const [filter, setFilter] = useState<Durability | 'all'>('all');

  const diff = useMemo(
    () => diffStorage(before, after, { ttlLedgers }),
    [before, after, ttlLedgers],
  );

  const visible = useMemo(
    () =>
      filter === 'all'
        ? diff.entries
        : diff.entries.filter((e) => e.durability === filter),
    [diff.entries, filter],
  );

  return (
    <div className="storage-diff" data-testid="storage-diff-viewer">
      <header className="storage-diff__header">
        <div className="header-icon-wrapper">
          <Database size={22} className="header-icon" aria-hidden="true" />
        </div>
        <div>
          <h2>Storage Diff</h2>
          <p>Footprint changes from the last simulation</p>
        </div>
      </header>

      <div className="storage-diff__summary" data-testid="storage-diff-summary">
        {diff.summary.map((s) => (
          <button
            key={s.durability}
            type="button"
            className={`summary-card ${filter === s.durability ? 'is-active' : ''}`}
            data-testid={`summary-${s.durability}`}
            aria-pressed={filter === s.durability}
            onClick={() =>
              setFilter((current) =>
                current === s.durability ? 'all' : s.durability,
              )
            }
          >
            <span className="summary-card__title">
              {DURABILITY_LABEL[s.durability]}
            </span>
            <span className="summary-card__counts">
              <span className="is-created">+{s.created}</span>
              <span className="is-modified">~{s.modified}</span>
              <span className="is-deleted">-{s.deleted}</span>
            </span>
            <span className="summary-card__bytes">
              {formatBytes(s.deltaBytes, true)}
            </span>
          </button>
        ))}
      </div>

      {diff.entries.length === 0 ? (
        <p className="storage-diff__empty" data-testid="storage-diff-empty">
          This transaction did not change any storage entries.
        </p>
      ) : (
        <table className="storage-diff__table">
          <thead>
            <tr>
              <th scope="col">
                <span className="visually-hidden">Expand</span>
              </th>
              <th scope="col">Change</th>
              <th scope="col">Key</th>
              <th scope="col">Durability</th>
              <th scope="col">Size</th>
              <th scope="col">Δ Bytes</th>
              <th scope="col">Rent</th>
            </tr>
          </thead>
          <tbody>
            {visible.map((entry) => (
              <DiffRow
                key={`${entry.durability}-${entry.key}`}
                entry={entry}
              />
            ))}
          </tbody>
          <tfoot>
            <tr data-testid="storage-diff-totals">
              <td colSpan={5}>Total</td>
              <td className="diff-row__numeric">
                {formatBytes(diff.totalDeltaBytes, true)}
              </td>
              <td className="diff-row__numeric">
                {formatStroops(diff.totalRentStroops)}
              </td>
            </tr>
          </tfoot>
        </table>
      )}
    </div>
  );
};

export default StorageDiffViewer;
