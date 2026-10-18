import React, { useMemo, useState } from 'react';
import {
  ArrowDownToLine,
  ArrowUpFromLine,
  ChevronLeft,
  ChevronRight,
  History,
  RotateCcw,
  SkipForward,
} from 'lucide-react';
import './TransactionTimeTravelDebugger.css';
import {
  SAMPLE_TRACE,
  callStackAt,
  diffState,
  previousFrameForContract,
  stepBack,
  stepInto,
  stepOut,
  stepOver,
} from './executionTrace';
import type { TraceFrame } from './executionTrace';

interface TransactionTimeTravelDebuggerProps {
  frames?: TraceFrame[];
}

/**
 * Steps forward and backward through a simulated transaction, showing the
 * locals and storage in scope at each frame.
 *
 * Every frame carries its own complete state rather than being replayed from a
 * log, so stepping backward is exact rather than an approximate undo.
 */
export const TransactionTimeTravelDebugger: React.FC<TransactionTimeTravelDebuggerProps> = ({
  frames = SAMPLE_TRACE,
}) => {
  const [current, setCurrent] = useState(0);

  const frame = frames[current];
  // Diffed against this contract's own previous frame, not simply the frame
  // before, so a nested call to another contract does not reset the diff.
  const previous = useMemo(() => previousFrameForContract(frames, current), [frames, current]);

  const delta = useMemo(
    () => diffState(previous?.storage, frame?.storage ?? {}),
    [previous, frame],
  );
  const stack = useMemo(() => callStackAt(frames, current), [frames, current]);

  const atStart = current === 0;
  const atEnd = current === frames.length - 1;

  if (!frame) {
    return (
      <div className="ttd-container" data-testid="time-travel-debugger">
        <p className="ttd-empty" data-testid="ttd-empty">
          No execution trace to replay.
        </p>
      </div>
    );
  }

  const storageClass = (key: string) => {
    if (delta.added.includes(key)) return 'added';
    if (delta.changed.includes(key)) return 'changed';
    return '';
  };

  return (
    <div className="ttd-container" data-testid="time-travel-debugger">
      <header className="ttd-header">
        <div className="ttd-title">
          <History size={20} />
          <div>
            <h2>Time-Travel Debugger</h2>
            <p>Replay a simulated transaction frame by frame</p>
          </div>
        </div>
        <span className="ttd-counter" data-testid="ttd-position">
          Step {current + 1} of {frames.length}
        </span>
      </header>

      <div className="ttd-controls" role="toolbar" aria-label="Debugger controls">
        <button
          type="button"
          className="ttd-btn"
          data-testid="step-back"
          disabled={atStart}
          onClick={() => setCurrent((i) => stepBack(frames, i))}
        >
          <ChevronLeft size={15} /> Step Back
        </button>
        <button
          type="button"
          className="ttd-btn"
          data-testid="step-into"
          disabled={atEnd}
          onClick={() => setCurrent((i) => stepInto(frames, i))}
        >
          <ArrowDownToLine size={15} /> Step Into
        </button>
        <button
          type="button"
          className="ttd-btn"
          data-testid="step-over"
          disabled={atEnd}
          onClick={() => setCurrent((i) => stepOver(frames, i))}
        >
          <ChevronRight size={15} /> Step Over
        </button>
        <button
          type="button"
          className="ttd-btn"
          data-testid="step-out"
          disabled={atEnd}
          onClick={() => setCurrent((i) => stepOut(frames, i))}
        >
          <ArrowUpFromLine size={15} /> Step Out
        </button>
        <button
          type="button"
          className="ttd-btn"
          data-testid="run-to-end"
          disabled={atEnd}
          onClick={() => setCurrent(frames.length - 1)}
        >
          <SkipForward size={15} /> Run to End
        </button>
        <button
          type="button"
          className="ttd-btn ghost"
          data-testid="restart"
          disabled={atStart}
          onClick={() => setCurrent(0)}
        >
          <RotateCcw size={15} /> Restart
        </button>
      </div>

      <input
        type="range"
        className="ttd-scrubber"
        min={0}
        max={frames.length - 1}
        value={current}
        aria-label="Execution timeline"
        data-testid="ttd-scrubber"
        onChange={(e) => setCurrent(Number(e.target.value))}
      />

      <section className="ttd-current" data-testid="ttd-current-frame">
        <div className="ttd-current-head">
          <code className="ttd-fn">
            {frame.contractId}.{frame.functionName}
          </code>
          <span className="ttd-depth" data-testid="ttd-depth">
            depth {frame.depth}
          </span>
          <span className="ttd-cpu" data-testid="ttd-cpu">
            {frame.cpuConsumed.toLocaleString()} CPU
          </span>
        </div>
        <p className="ttd-operation" data-testid="ttd-operation">
          {frame.operation}
        </p>
      </section>

      <div className="ttd-panels">
        <section className="ttd-panel">
          <h3>Locals</h3>
          {Object.keys(frame.locals).length === 0 ? (
            <p className="ttd-empty">No locals in scope.</p>
          ) : (
            <table className="ttd-table" data-testid="ttd-locals">
              <tbody>
                {Object.entries(frame.locals).map(([key, value]) => (
                  <tr key={key} data-testid={`local-${key}`}>
                    <th scope="row">{key}</th>
                    <td>{value}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </section>

        <section className="ttd-panel">
          <h3>Storage</h3>
          <table className="ttd-table" data-testid="ttd-storage">
            <tbody>
              {Object.entries(frame.storage).map(([key, value]) => (
                <tr key={key} className={storageClass(key)} data-testid={`storage-${key}`}>
                  <th scope="row">{key}</th>
                  <td>
                    {value}
                    {delta.changed.includes(key) && previous && (
                      <span className="ttd-was" data-testid={`storage-was-${key}`}>
                        was {previous.storage[key]}
                      </span>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      </div>

      <div className="ttd-panels">
        <section className="ttd-panel">
          <h3>Call stack</h3>
          <ol className="ttd-stack" data-testid="ttd-stack">
            {stack.map((entry) => (
              <li key={entry.index} data-testid={`stack-${entry.depth}`}>
                <code>
                  {entry.contractId}.{entry.functionName}
                </code>
              </li>
            ))}
          </ol>
        </section>

        <section className="ttd-panel">
          <h3>Events</h3>
          {frame.events.length === 0 ? (
            <p className="ttd-empty" data-testid="ttd-no-events">
              No events emitted at this step.
            </p>
          ) : (
            <ul className="ttd-events" data-testid="ttd-events">
              {frame.events.map((event) => (
                <li key={event}>
                  <code>{event}</code>
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>

      <section className="ttd-panel">
        <h3>Frames</h3>
        <ol className="ttd-frame-list" data-testid="ttd-frame-list">
          {frames.map((f) => (
            <li key={f.index}>
              <button
                type="button"
                className={`ttd-frame-btn ${f.index === current ? 'active' : ''}`}
                style={{ paddingLeft: `${12 + f.depth * 16}px` }}
                aria-current={f.index === current ? 'step' : undefined}
                data-testid={`frame-${f.index}`}
                onClick={() => setCurrent(f.index)}
              >
                <span className="ttd-frame-index">{f.index}</span>
                <code>{f.functionName}</code>
                <span className="ttd-frame-op">{f.operation}</span>
              </button>
            </li>
          ))}
        </ol>
      </section>
    </div>
  );
};

export default TransactionTimeTravelDebugger;
