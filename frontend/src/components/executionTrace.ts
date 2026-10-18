/**
 * Execution frame model and stepping logic for the time-travel debugger.
 *
 * Kept separate from the component so the navigation semantics — which are the
 * part that is easy to get subtly wrong — can be tested directly.
 */

export interface TraceFrame {
  /** Position in the trace; also the id used by the UI. */
  index: number;
  /** Call depth. 0 is the entry point; deeper frames are nested calls. */
  depth: number;
  contractId: string;
  functionName: string;
  /** What this frame does, shown as the step description. */
  operation: string;
  /** Locals in scope at this point. */
  locals: Record<string, string>;
  /** Full contract storage as of this frame. */
  storage: Record<string, string>;
  /** Events emitted by this frame, if any. */
  events: string[];
  /** Cumulative CPU instructions at this frame. */
  cpuConsumed: number;
}

export interface StateDelta {
  added: string[];
  changed: string[];
  removed: string[];
}

/** Frames reachable by "step forward" — simply the next frame. */
export function stepForward(frames: TraceFrame[], current: number): number {
  return Math.min(current + 1, frames.length - 1);
}

/** Frames reachable by "step back" — the previous frame. */
export function stepBack(frames: TraceFrame[], current: number): number {
  void frames;
  return Math.max(current - 1, 0);
}

/**
 * Step into: enter the very next frame whatever its depth. This is the same
 * index as stepping forward, and exists as its own control so the debugger
 * reads like the ones developers already use.
 */
export function stepInto(frames: TraceFrame[], current: number): number {
  return stepForward(frames, current);
}

/**
 * Step over: advance to the next frame at the same depth or shallower, so a
 * nested call is executed but not descended into. If the current frame is the
 * last at its depth, this lands on the frame that unwinds past it.
 */
export function stepOver(frames: TraceFrame[], current: number): number {
  const currentDepth = frames[current]?.depth ?? 0;
  for (let i = current + 1; i < frames.length; i++) {
    if (frames[i].depth <= currentDepth) return i;
  }
  return frames.length - 1;
}

/**
 * Step out: run to the first frame shallower than the current one, i.e. return
 * to the caller. From depth 0 there is no caller, so this runs to the end.
 */
export function stepOut(frames: TraceFrame[], current: number): number {
  const currentDepth = frames[current]?.depth ?? 0;
  if (currentDepth === 0) return frames.length - 1;
  for (let i = current + 1; i < frames.length; i++) {
    if (frames[i].depth < currentDepth) return i;
  }
  return frames.length - 1;
}

/**
 * Which storage keys differ between two frames. Used to highlight what a step
 * actually changed rather than making the reader diff two tables by eye.
 */
export function diffState(
  previous: Record<string, string> | undefined,
  next: Record<string, string>,
): StateDelta {
  const before = previous ?? {};
  const added: string[] = [];
  const changed: string[] = [];
  const removed: string[] = [];

  Object.keys(next).forEach((key) => {
    if (!(key in before)) added.push(key);
    else if (before[key] !== next[key]) changed.push(key);
  });

  Object.keys(before).forEach((key) => {
    if (!(key in next)) removed.push(key);
  });

  return { added, changed, removed };
}

/**
 * The most recent earlier frame belonging to the same contract.
 *
 * Storage is per-contract, so diffing a frame against whatever ran immediately
 * before it would compare two different contracts' storage and report every key
 * as newly added. Comparing against the same contract's previous frame shows
 * what this contract actually changed.
 */
export function previousFrameForContract(
  frames: TraceFrame[],
  current: number,
): TraceFrame | undefined {
  const frame = frames[current];
  if (!frame) return undefined;

  for (let i = current - 1; i >= 0; i--) {
    if (frames[i].contractId === frame.contractId) return frames[i];
  }
  return undefined;
}

/** The call stack at a frame, outermost first. */
export function callStackAt(frames: TraceFrame[], current: number): TraceFrame[] {
  if (!frames[current]) return [];

  const stack: TraceFrame[] = [];
  let depth = frames[current].depth;

  for (let i = current; i >= 0; i--) {
    if (frames[i].depth === depth) {
      stack.unshift(frames[i]);
      depth -= 1;
      if (depth < 0) break;
    }
  }

  return stack;
}

/**
 * Adapt the flat `traceLogs` produced by the existing transaction simulator
 * into frames, so a simulation can be replayed without changing its shape.
 *
 * Logs are expected as `depth|function|operation`; anything that does not match
 * is treated as a depth-0 step so an unrecognised line is still shown rather
 * than dropped.
 */
export function framesFromTraceLogs(
  logs: string[],
  contractId: string,
  cpuTotal = 0,
): TraceFrame[] {
  const perFrame = logs.length > 0 ? Math.round(cpuTotal / logs.length) : 0;

  return logs.map((log, index) => {
    const parts = log.split('|');
    const structured = parts.length >= 3;
    const depth = structured ? Math.max(0, Number.parseInt(parts[0], 10) || 0) : 0;

    return {
      index,
      depth,
      contractId,
      functionName: structured ? parts[1].trim() : 'trace',
      operation: structured ? parts.slice(2).join('|').trim() : log,
      locals: {},
      storage: {},
      events: [],
      cpuConsumed: perFrame * (index + 1),
    };
  });
}

/**
 * A worked escrow release, including a nested token transfer, so the debugger
 * has something with real call depth to step through.
 */
export const SAMPLE_TRACE: TraceFrame[] = [
  {
    index: 0,
    depth: 0,
    contractId: 'CESCROW',
    functionName: 'release',
    operation: 'Entry — authenticate caller',
    locals: { caller: 'GDEPOSITOR…', escrow_id: '42' },
    storage: { 'escrow:42:status': 'Funded', 'escrow:42:amount': '2500', 'escrow:42:beneficiary': 'GBENEF…' },
    events: [],
    cpuConsumed: 1_240,
  },
  {
    index: 1,
    depth: 0,
    contractId: 'CESCROW',
    functionName: 'release',
    operation: 'Load escrow record from storage',
    locals: { caller: 'GDEPOSITOR…', escrow_id: '42', amount: '2500' },
    storage: { 'escrow:42:status': 'Funded', 'escrow:42:amount': '2500', 'escrow:42:beneficiary': 'GBENEF…' },
    events: [],
    cpuConsumed: 3_910,
  },
  {
    index: 2,
    depth: 0,
    contractId: 'CESCROW',
    functionName: 'release',
    operation: 'Guard — require status == Funded',
    locals: { caller: 'GDEPOSITOR…', escrow_id: '42', amount: '2500', status: 'Funded' },
    storage: { 'escrow:42:status': 'Funded', 'escrow:42:amount': '2500', 'escrow:42:beneficiary': 'GBENEF…' },
    events: [],
    cpuConsumed: 4_480,
  },
  {
    index: 3,
    depth: 1,
    contractId: 'CTOKEN',
    functionName: 'transfer',
    operation: 'Cross-contract call — debit escrow balance',
    locals: { from: 'CESCROW', to: 'GBENEF…', amount: '2500' },
    storage: { 'balance:CESCROW': '2500', 'balance:GBENEF…': '0' },
    events: [],
    cpuConsumed: 9_120,
  },
  {
    index: 4,
    depth: 1,
    contractId: 'CTOKEN',
    functionName: 'transfer',
    operation: 'Credit beneficiary and emit transfer event',
    locals: { from: 'CESCROW', to: 'GBENEF…', amount: '2500' },
    storage: { 'balance:CESCROW': '0', 'balance:GBENEF…': '2500' },
    events: ['transfer(CESCROW, GBENEF…, 2500)'],
    cpuConsumed: 13_760,
  },
  {
    index: 5,
    depth: 0,
    contractId: 'CESCROW',
    functionName: 'release',
    operation: 'Mark escrow released',
    locals: { caller: 'GDEPOSITOR…', escrow_id: '42', amount: '2500', status: 'Released' },
    storage: { 'escrow:42:status': 'Released', 'escrow:42:amount': '0', 'escrow:42:beneficiary': 'GBENEF…' },
    events: [],
    cpuConsumed: 15_030,
  },
  {
    index: 6,
    depth: 0,
    contractId: 'CESCROW',
    functionName: 'release',
    operation: 'Emit release event and return',
    locals: { caller: 'GDEPOSITOR…', escrow_id: '42', amount: '2500', status: 'Released' },
    storage: { 'escrow:42:status': 'Released', 'escrow:42:amount': '0', 'escrow:42:beneficiary': 'GBENEF…' },
    events: ['escrow_released(42, GBENEF…, 2500)'],
    cpuConsumed: 16_400,
  },
];
