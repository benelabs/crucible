import React, { useMemo, useState } from 'react';
import { Copy, GitBranch, RotateCcw, Workflow } from 'lucide-react';
import './ContractFlowchartVisualizer.css';
import { layoutFlowchart, permittedTransitions, toMermaid, toSvg } from './contractFlowchart';
import type { ContractFlowchartSpec } from './contractFlowchart';

interface ContractFlowchartVisualizerProps {
  spec?: ContractFlowchartSpec;
  /** Starting state; defaults to the spec's initial state. */
  initialState?: string;
}

/** An escrow lifecycle — the kind of multi-stage contract this exists for. */
export const SAMPLE_FLOWCHART: ContractFlowchartSpec = {
  name: 'Escrow',
  initial: 'Uninitialized',
  states: ['Uninitialized', 'Initialized', 'Funded', 'Disputed', 'Released', 'Refunded'],
  terminal: ['Released', 'Refunded'],
  transitions: [
    { name: 'initialize', from: 'Uninitialized', to: 'Initialized', requires: 'admin' },
    { name: 'fund', from: 'Initialized', to: 'Funded', requires: 'depositor' },
    { name: 'top_up', from: 'Funded', to: 'Funded', requires: 'depositor' },
    { name: 'release', from: 'Funded', to: 'Released', requires: 'depositor' },
    { name: 'dispute', from: 'Funded', to: 'Disputed', requires: 'either party' },
    { name: 'resolve_for_seller', from: 'Disputed', to: 'Released', requires: 'arbiter' },
    { name: 'resolve_for_buyer', from: 'Disputed', to: 'Refunded', requires: 'arbiter' },
  ],
};

/**
 * Renders a contract's state machine as a diagram, highlighting the active
 * state and the transitions currently permitted from it.
 *
 * The SVG is generated directly rather than through a rendering library, so the
 * component has no runtime dependency and the markup can be asserted in tests.
 * Mermaid source is offered alongside for pasting into docs.
 */
export const ContractFlowchartVisualizer: React.FC<ContractFlowchartVisualizerProps> = ({
  spec = SAMPLE_FLOWCHART,
  initialState,
}) => {
  const [activeState, setActiveState] = useState(initialState ?? spec.initial);
  const [showMermaid, setShowMermaid] = useState(false);
  const [copied, setCopied] = useState(false);

  const layout = useMemo(() => layoutFlowchart(spec, activeState), [spec, activeState]);
  const svgMarkup = useMemo(() => toSvg(spec, activeState), [spec, activeState]);
  const mermaid = useMemo(() => toMermaid(spec, activeState), [spec, activeState]);
  const available = useMemo(() => permittedTransitions(spec, activeState), [spec, activeState]);

  const isTerminal = (spec.terminal ?? []).includes(activeState);

  const copyMermaid = async () => {
    try {
      await navigator.clipboard?.writeText(mermaid);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard is unavailable in some browsers and in tests; the source is
      // on screen either way, so this is not worth surfacing as an error.
      setCopied(false);
    }
  };

  return (
    <div className="cfv-container" data-testid="contract-flowchart">
      <header className="cfv-header">
        <div className="cfv-title">
          <Workflow size={20} />
          <div>
            <h2>State Machine Visualizer</h2>
            <p>{spec.name} lifecycle, generated from its interface</p>
          </div>
        </div>
        <div className="cfv-header-actions">
          <button
            type="button"
            className="cfv-ghost-btn"
            data-testid="toggle-mermaid"
            onClick={() => setShowMermaid((v) => !v)}
          >
            <GitBranch size={14} /> {showMermaid ? 'Hide' : 'Show'} Mermaid
          </button>
          <button
            type="button"
            className="cfv-ghost-btn"
            data-testid="reset-state"
            onClick={() => setActiveState(initialState ?? spec.initial)}
          >
            <RotateCcw size={14} /> Reset
          </button>
        </div>
      </header>

      <div className="cfv-diagram-wrap">
        <div
          className="cfv-diagram"
          data-testid="flowchart-svg"
          role="figure"
          aria-label={`${spec.name} state machine, currently ${activeState}`}
          dangerouslySetInnerHTML={{ __html: svgMarkup }}
        />
      </div>

      <div className="cfv-panels">
        <section className="cfv-panel">
          <h3>States</h3>
          <div className="cfv-state-list">
            {layout.nodes.map((node) => (
              <button
                key={node.id}
                type="button"
                className={`cfv-state-chip ${node.id === activeState ? 'active' : ''}`}
                aria-pressed={node.id === activeState}
                data-testid={`state-${node.id}`}
                onClick={() => setActiveState(node.id)}
              >
                {node.label}
                {node.isTerminal && <span className="cfv-terminal-dot" aria-label="terminal state" />}
              </button>
            ))}
          </div>
        </section>

        <section className="cfv-panel">
          <h3>
            Permitted from <code data-testid="active-state">{activeState}</code>
          </h3>
          {available.length === 0 ? (
            <p className="cfv-empty" data-testid="no-transitions">
              {isTerminal
                ? 'Terminal state — no further transitions.'
                : 'No transitions defined from this state.'}
            </p>
          ) : (
            <ul className="cfv-transition-list">
              {available.map((t) => (
                <li key={`${t.name}-${t.to}`}>
                  <button
                    type="button"
                    className="cfv-transition-btn"
                    data-testid={`transition-${t.name}`}
                    onClick={() => setActiveState(t.to)}
                  >
                    <span className="cfv-transition-name">{t.name}</span>
                    <span className="cfv-transition-target">→ {t.to}</span>
                    {t.requires && <span className="cfv-transition-role">{t.requires}</span>}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>

      {showMermaid && (
        <section className="cfv-mermaid">
          <div className="cfv-mermaid-head">
            <span>Mermaid source</span>
            <button type="button" className="cfv-ghost-btn" data-testid="copy-mermaid" onClick={copyMermaid}>
              <Copy size={13} /> {copied ? 'Copied' : 'Copy'}
            </button>
          </div>
          <pre data-testid="mermaid-source">{mermaid}</pre>
        </section>
      )}
    </div>
  );
};

export default ContractFlowchartVisualizer;
