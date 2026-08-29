/**
 * Pure model behind {@link StorageDiffViewer} (issue #889).
 *
 * Kept separate from the component so the diff algorithm and the rent
 * arithmetic can be tested without rendering anything, and so a real
 * simulation backend can feed the same shapes.
 */

/** Soroban's three storage durabilities. */
export type Durability = 'instance' | 'persistent' | 'temporary';

export type ChangeKind = 'created' | 'modified' | 'deleted' | 'unchanged';

export interface StorageEntry {
  /** Contract the entry belongs to. */
  contractId: string;
  durability: Durability;
  /** Human-readable key, e.g. `Balance(GBA2K…)`. */
  key: string;
  /** Serialised value preview. */
  value: string;
  /** Size of the entry in bytes, as it counts towards rent. */
  sizeBytes: number;
}

export interface StorageDiffEntry {
  contractId: string;
  durability: Durability;
  key: string;
  kind: ChangeKind;
  before: string | null;
  after: string | null;
  beforeBytes: number;
  afterBytes: number;
  /** Positive when the entry grew, negative when it shrank. */
  deltaBytes: number;
  /** Rent for this entry over `ttlLedgers`, in stroops. */
  rentStroops: number;
}

export interface DurabilitySummary {
  durability: Durability;
  created: number;
  modified: number;
  deleted: number;
  deltaBytes: number;
  rentStroops: number;
}

export interface StorageDiff {
  entries: StorageDiffEntry[];
  summary: DurabilitySummary[];
  totalDeltaBytes: number;
  totalRentStroops: number;
}

/**
 * Rent cost per byte per ledger, in stroops, by durability.
 *
 * Temporary entries are far cheaper because the network reclaims them, and
 * instance storage rides along with the contract instance rather than being
 * rented per entry — modelled here as the same rate as persistent, since it
 * is charged on the same footprint.
 */
export const RENT_RATE_STROOPS: Record<Durability, number> = {
  instance: 0.0002,
  persistent: 0.0002,
  temporary: 0.00002,
};

/** Default TTL used for rent projection, in ledgers (~30 days at 5s). */
export const DEFAULT_TTL_LEDGERS = 518_400;

function entryId(entry: Pick<StorageEntry, 'contractId' | 'durability' | 'key'>) {
  return `${entry.contractId}::${entry.durability}::${entry.key}`;
}

/**
 * Rent for a footprint change.
 *
 * A deletion is charged zero rather than a negative amount: the network does
 * not refund rent already paid, so showing a credit would misrepresent what
 * the transaction costs. Only bytes that are actually being rented going
 * forward are billed, which is why this uses `afterBytes` and not the delta.
 */
export function calculateRent(
  afterBytes: number,
  durability: Durability,
  ttlLedgers: number = DEFAULT_TTL_LEDGERS,
): number {
  if (afterBytes <= 0) return 0;
  return Math.round(afterBytes * RENT_RATE_STROOPS[durability] * ttlLedgers);
}

/**
 * Diff two storage footprints.
 *
 * Entries are matched on contract + durability + key, all three: the same key
 * written to persistent and temporary storage is two different entries, and
 * treating them as one would report a spurious modification.
 *
 * `includeUnchanged` is off by default — a footprint is mostly untouched
 * entries, and burying four real changes among two hundred rows is what makes
 * a raw footprint dump unreadable in the first place.
 */
export function diffStorage(
  before: StorageEntry[],
  after: StorageEntry[],
  options: { ttlLedgers?: number; includeUnchanged?: boolean } = {},
): StorageDiff {
  const ttl = options.ttlLedgers ?? DEFAULT_TTL_LEDGERS;
  const includeUnchanged = options.includeUnchanged ?? false;

  const beforeMap = new Map(before.map((e) => [entryId(e), e]));
  const afterMap = new Map(after.map((e) => [entryId(e), e]));

  const ids = new Set<string>([...beforeMap.keys(), ...afterMap.keys()]);
  const entries: StorageDiffEntry[] = [];

  for (const id of ids) {
    const b = beforeMap.get(id);
    const a = afterMap.get(id);
    const ref = (a ?? b)!;

    let kind: ChangeKind;
    if (!b) kind = 'created';
    else if (!a) kind = 'deleted';
    else if (b.value !== a.value || b.sizeBytes !== a.sizeBytes) kind = 'modified';
    else kind = 'unchanged';

    if (kind === 'unchanged' && !includeUnchanged) continue;

    const beforeBytes = b?.sizeBytes ?? 0;
    const afterBytes = a?.sizeBytes ?? 0;

    entries.push({
      contractId: ref.contractId,
      durability: ref.durability,
      key: ref.key,
      kind,
      before: b?.value ?? null,
      after: a?.value ?? null,
      beforeBytes,
      afterBytes,
      deltaBytes: afterBytes - beforeBytes,
      rentStroops: calculateRent(afterBytes, ref.durability, ttl),
    });
  }

  // Created first, then modified, then deleted, then by key — the order a
  // reader scans for "what did this transaction actually do".
  const order: Record<ChangeKind, number> = {
    created: 0,
    modified: 1,
    deleted: 2,
    unchanged: 3,
  };
  entries.sort(
    (x, y) => order[x.kind] - order[y.kind] || x.key.localeCompare(y.key),
  );

  const durabilities: Durability[] = ['instance', 'persistent', 'temporary'];
  const summary = durabilities.map((durability) => {
    const rows = entries.filter((e) => e.durability === durability);
    return {
      durability,
      created: rows.filter((e) => e.kind === 'created').length,
      modified: rows.filter((e) => e.kind === 'modified').length,
      deleted: rows.filter((e) => e.kind === 'deleted').length,
      deltaBytes: rows.reduce((sum, e) => sum + e.deltaBytes, 0),
      rentStroops: rows.reduce((sum, e) => sum + e.rentStroops, 0),
    };
  });

  return {
    entries,
    summary,
    totalDeltaBytes: entries.reduce((sum, e) => sum + e.deltaBytes, 0),
    totalRentStroops: entries.reduce((sum, e) => sum + e.rentStroops, 0),
  };
}

/** Format a byte count for display, signed for deltas. */
export function formatBytes(bytes: number, signed = false): string {
  const sign = signed && bytes > 0 ? '+' : '';
  if (Math.abs(bytes) < 1024) return `${sign}${bytes} B`;
  return `${sign}${(bytes / 1024).toFixed(2)} KiB`;
}

/** Format stroops as XLM when large enough to be worth reading that way. */
export function formatStroops(stroops: number): string {
  if (stroops < 10_000) return `${stroops.toLocaleString()} stroops`;
  return `${(stroops / 10_000_000).toFixed(4)} XLM`;
}
