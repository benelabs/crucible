import { describe, expect, it } from 'vitest';
import {
  SAMPLE_TRACE,
  TraceFrame,
  callStackAt,
  diffState,
  framesFromTraceLogs,
  previousFrameForContract,
  stepBack,
  stepForward,
  stepInto,
  stepOut,
  stepOver,
} from './executionTrace';

/** depth-only fixture: 0,0,1,2,1,0 */
const FRAMES: TraceFrame[] = [0, 0, 1, 2, 1, 0].map((depth, index) => ({
  index,
  depth,
  contractId: `C${depth}`,
  functionName: `fn${index}`,
  operation: `op ${index}`,
  locals: {},
  storage: {},
  events: [],
  cpuConsumed: index * 100,
}));

describe('stepForward / stepBack', () => {
  it('moves one frame at a time', () => {
    expect(stepForward(FRAMES, 0)).toBe(1);
    expect(stepBack(FRAMES, 3)).toBe(2);
  });

  it('clamps at both ends instead of running off the trace', () => {
    expect(stepForward(FRAMES, FRAMES.length - 1)).toBe(FRAMES.length - 1);
    expect(stepBack(FRAMES, 0)).toBe(0);
  });

  it('round-trips back to where it started', () => {
    expect(stepBack(FRAMES, stepForward(FRAMES, 2))).toBe(2);
  });
});

describe('stepInto', () => {
  it('descends into the nested call', () => {
    // frame 1 is depth 0; frame 2 is depth 1
    expect(stepInto(FRAMES, 1)).toBe(2);
  });
});

describe('stepOver', () => {
  it('skips a nested call and lands at the same depth', () => {
    // from frame 1 (depth 0) the next depth-0 frame is index 5
    expect(stepOver(FRAMES, 1)).toBe(5);
  });

  it('advances normally when the next frame is not deeper', () => {
    expect(stepOver(FRAMES, 0)).toBe(1);
  });

  it('runs to the end when nothing at that depth follows', () => {
    expect(stepOver(FRAMES, 5)).toBe(FRAMES.length - 1);
  });
});

describe('stepOut', () => {
  it('returns to the caller frame', () => {
    // frame 3 is depth 2; the next shallower frame is index 4 (depth 1)
    expect(stepOut(FRAMES, 3)).toBe(4);
  });

  it('runs to the end from the outermost depth', () => {
    expect(stepOut(FRAMES, 0)).toBe(FRAMES.length - 1);
  });
});

describe('diffState', () => {
  it('reports added, changed and removed keys', () => {
    const delta = diffState({ a: '1', b: '2' }, { b: '3', c: '4' });
    expect(delta).toEqual({ added: ['c'], changed: ['b'], removed: ['a'] });
  });

  it('reports nothing when state is unchanged', () => {
    expect(diffState({ a: '1' }, { a: '1' })).toEqual({ added: [], changed: [], removed: [] });
  });

  it('treats a missing previous frame as everything being new', () => {
    expect(diffState(undefined, { a: '1' })).toEqual({ added: ['a'], changed: [], removed: [] });
  });
});

describe('previousFrameForContract', () => {
  it('skips over frames belonging to another contract', () => {
    // SAMPLE_TRACE frame 5 is CESCROW; frames 3-4 are the nested CTOKEN call.
    expect(previousFrameForContract(SAMPLE_TRACE, 5)?.index).toBe(2);
  });

  it('returns the immediately previous frame within one contract', () => {
    expect(previousFrameForContract(SAMPLE_TRACE, 2)?.index).toBe(1);
  });

  it('returns undefined for the first frame of a contract', () => {
    expect(previousFrameForContract(SAMPLE_TRACE, 0)).toBeUndefined();
    expect(previousFrameForContract(SAMPLE_TRACE, 3)).toBeUndefined();
  });
});

describe('callStackAt', () => {
  it('returns the frame itself at the outermost depth', () => {
    expect(callStackAt(FRAMES, 0).map((f) => f.index)).toEqual([0]);
  });

  it('returns nothing when the index is outside the trace', () => {
    expect(callStackAt([], 0)).toEqual([]);
    expect(callStackAt(FRAMES, 99)).toEqual([]);
  });

  it('walks back through the callers of a nested frame', () => {
    // frame 3 (depth 2) is called from frame 2 (depth 1), itself from frame 1
    expect(callStackAt(FRAMES, 3).map((f) => f.index)).toEqual([1, 2, 3]);
  });
});

describe('framesFromTraceLogs', () => {
  it('parses structured depth|function|operation logs', () => {
    const frames = framesFromTraceLogs(['0|release|entry', '1|transfer|debit'], 'CESCROW', 200);
    expect(frames[0]).toMatchObject({ depth: 0, functionName: 'release', operation: 'entry' });
    expect(frames[1]).toMatchObject({ depth: 1, functionName: 'transfer', operation: 'debit' });
  });

  it('keeps an unstructured line as a depth-0 frame rather than dropping it', () => {
    const frames = framesFromTraceLogs(['just a log line'], 'C');
    expect(frames).toHaveLength(1);
    expect(frames[0]).toMatchObject({ depth: 0, operation: 'just a log line' });
  });

  it('spreads the CPU total across frames cumulatively', () => {
    const frames = framesFromTraceLogs(['0|a|x', '0|b|y'], 'C', 200);
    expect(frames[0].cpuConsumed).toBe(100);
    expect(frames[1].cpuConsumed).toBe(200);
  });

  it('handles an empty log list', () => {
    expect(framesFromTraceLogs([], 'C', 100)).toEqual([]);
  });
});

describe('SAMPLE_TRACE', () => {
  it('is indexed consecutively', () => {
    SAMPLE_TRACE.forEach((frame, index) => expect(frame.index).toBe(index));
  });

  it('contains a nested cross-contract call', () => {
    expect(SAMPLE_TRACE.some((f) => f.depth === 1)).toBe(true);
  });

  it('never decreases cumulative CPU', () => {
    for (let i = 1; i < SAMPLE_TRACE.length; i++) {
      expect(SAMPLE_TRACE[i].cpuConsumed).toBeGreaterThanOrEqual(SAMPLE_TRACE[i - 1].cpuConsumed);
    }
  });
});
