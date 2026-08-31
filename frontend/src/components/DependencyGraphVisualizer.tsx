import React, { useMemo, useState } from 'react';
import { ShieldAlert, AlertTriangle, RefreshCw, GitBranch } from 'lucide-react';
import {
  SAMPLE_DEP_GRAPH,
  SEVERITY_COLORS,
  computeLayout,
  detectCycles,
  type DepGraphData,
  type DepNode,
} from './dependencyGraphData';
import './DependencyGraphVisualizer.css';

interface DependencyGraphVisualizerProps {
  data?: DepGraphData;
}

export const DependencyGraphVisualizer: React.FC<DependencyGraphVisualizerProps> = ({
  data = SAMPLE_DEP_GRAPH,
}) => {
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [layoutSeed, setLayoutSeed] = useState(1337);

  const layout = useMemo(() => computeLayout(data, layoutSeed), [data, layoutSeed]);
  const cyclesDetected = useMemo(() => detectCycles(data), [data]);
  const nodeById = useMemo(() => {
    const m: Record<string, DepNode> = {};
    data.nodes.forEach((n) => (m[n.id] = n));
    return m;
  }, [data]);

  const selected = selectedId ? nodeById[selectedId] : null;

  const handleNodeClick = (id: string) => {
    setSelectedId(id);
  };

  return (
    <div className="dep-graph-container" data-testid="dependency-graph">
      <div className="dep-graph-header">
        <div className="header-icon-wrapper">
          <GitBranch className="header-icon" />
        </div>
        <div>
          <h2>Contract Dependency Tree Visualizer</h2>
          <p>Audit multi-crate dependency hierarchies and security advisory flags</p>
        </div>
        <button
          type="button"
          className="relayout-btn"
          onClick={() => setLayoutSeed((s) => s + 1)}
          data-testid="relayout-button"
        >
          <RefreshCw size={14} /> Re-run layout
        </button>
      </div>

      <div className="dep-graph-legend" aria-label="Severity legend">
        {(Object.keys(SEVERITY_COLORS) as DepNode['severity'][]).map((sev) => (
          <span className="legend-item" key={sev}>
            <span
              className="legend-swatch"
              style={{ background: SEVERITY_COLORS[sev] }}
            />
            {sev}
          </span>
        ))}
      </div>

      <div className="dep-graph-status-row">
        <span
          className={`cycle-badge ${cyclesDetected ? 'bad' : 'ok'}`}
          data-testid="cycle-badge"
        >
          {cyclesDetected ? (
            <>
              <AlertTriangle size={13} /> Circular dependency detected
            </>
          ) : (
            <>
              <ShieldAlert size={13} /> No circular dependencies
            </>
          )}
        </span>
      </div>

      <div className="dep-graph-body">
        <svg
          className="dep-graph-svg"
          viewBox={`0 0 ${800} ${520}`}
          role="img"
          aria-label="Dependency graph"
          data-testid="dep-graph-svg"
        >
          {data.edges.map((e, i) => {
            const a = layout[e.from];
            const b = layout[e.to];
            if (!a || !b) return null;
            const isCycle = e.from === e.to;
            return (
              <line
                key={`edge-${i}`}
                x1={a.x}
                y1={a.y}
                x2={b.x}
                y2={b.y}
                className={`dep-edge ${isCycle ? 'self-loop' : ''} ${
                  selectedId && (selectedId === e.from || selectedId === e.to)
                    ? 'edge-highlight'
                    : ''
                }`}
                data-testid={`dep-edge-${e.from}-${e.to}`}
              />
            );
          })}

          {data.nodes.map((n) => {
            const p = layout[n.id];
            const isSelected = selectedId === n.id;
            return (
              <g
                key={n.id}
                transform={`translate(${p.x}, ${p.y})`}
                className={`dep-node ${isSelected ? 'selected' : ''}`}
                data-testid={`dep-node-${n.id}`}
                data-severity={n.severity}
                data-update-status={n.updateStatus}
                onClick={() => handleNodeClick(n.id)}
                onKeyDown={(ev) => {
                  if (ev.key === 'Enter' || ev.key === ' ') {
                    ev.preventDefault();
                    handleNodeClick(n.id);
                  }
                }}
                tabIndex={0}
                role="button"
                aria-label={`${n.name} severity ${n.severity} status ${n.updateStatus}`}
                style={{ cursor: 'pointer' }}
              >
                <circle
                  r={isSelected ? 22 : 18}
                  fill={SEVERITY_COLORS[n.severity]}
                  stroke={isSelected ? '#ffffff' : '#0d1117'}
                  strokeWidth={isSelected ? 3 : 2}
                />
                {n.vulnerabilities > 0 && (
                  <text className="dep-node-badge" y={-26} textAnchor="middle">
                    {n.vulnerabilities}
                  </text>
                )}
                <text className="dep-node-label" y={34} textAnchor="middle">
                  {n.name}
                </text>
              </g>
            );
          })}
        </svg>

        <aside
          className="dep-detail-panel"
          data-testid="dep-detail-panel"
          aria-live="polite"
        >
          {selected ? (
            <>
              <h3 data-testid="dep-detail-name">{selected.name}</h3>
              <div className="dep-detail-meta">
                <span className="dep-detail-version">v{selected.version}</span>
                <span
                  className="dep-detail-sev"
                  style={{ background: SEVERITY_COLORS[selected.severity] }}
                  data-testid="dep-detail-severity"
                >
                  {selected.severity}
                </span>
                <span
                  className={`dep-detail-status status-${selected.updateStatus}`}
                  data-testid="dep-detail-status"
                >
                  {selected.updateStatus}
                </span>
              </div>
              <p className="dep-detail-desc" data-testid="dep-detail-desc">
                {selected.description}
              </p>
              <div className="dep-detail-stat">
                <span>Known vulnerabilities</span>
                <strong data-testid="dep-detail-vulns">{selected.vulnerabilities}</strong>
              </div>
              <ul className="dep-detail-deps">
                {data.edges
                  .filter((e) => e.from === selected.id && e.from !== e.to)
                  .map((e) => (
                    <li key={e.to} data-testid={`dep-detail-dep-${e.to}`}>
                      {nodeById[e.to]?.name ?? e.to}
                    </li>
                  ))}
              </ul>
            </>
          ) : (
            <div className="dep-detail-empty">
              <ShieldAlert size={28} />
              <p>Select a node to inspect crate details, advisories and dependencies.</p>
            </div>
          )}
        </aside>
      </div>
    </div>
  );
};

export default DependencyGraphVisualizer;
