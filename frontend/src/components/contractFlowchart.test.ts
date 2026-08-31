import { describe, expect, it } from 'vitest';
import {
  ContractFlowchartSpec,
  escapeXml,
  inferFlowchart,
  layoutFlowchart,
  permittedTransitions,
  toMermaid,
  toMermaidId,
  toSvg,
} from './contractFlowchart';

const SPEC: ContractFlowchartSpec = {
  name: 'Escrow',
  initial: 'Draft',
  states: ['Draft', 'Funded', 'Released'],
  terminal: ['Released'],
  transitions: [
    { name: 'fund', from: 'Draft', to: 'Funded', requires: 'depositor' },
    { name: 'top_up', from: 'Funded', to: 'Funded' },
    { name: 'release', from: 'Funded', to: 'Released' },
  ],
};

describe('escapeXml', () => {
  it('escapes characters that would break out of markup', () => {
    expect(escapeXml('<a & "b">')).toBe('&lt;a &amp; &quot;b&quot;&gt;');
  });
});

describe('toMermaidId', () => {
  it('strips characters Mermaid cannot use in an id', () => {
    expect(toMermaidId('In Progress')).toBe('In_Progress');
    expect(toMermaidId('a-b')).toBe('a_b');
  });

  it('prefixes ids that would start with a digit', () => {
    expect(toMermaidId('2FA')).toBe('s_2FA');
  });
});

describe('toMermaid', () => {
  it('emits a stateDiagram-v2 with an entry point', () => {
    const out = toMermaid(SPEC);
    expect(out.split('\n')[0]).toBe('stateDiagram-v2');
    expect(out).toContain('[*] --> Draft');
  });

  it('emits one edge per transition, labelled with the function', () => {
    const out = toMermaid(SPEC);
    expect(out).toContain('Draft --> Funded: fund (depositor)');
    expect(out).toContain('Funded --> Released: release');
  });

  it('marks terminal states as exiting to [*]', () => {
    expect(toMermaid(SPEC)).toContain('Released --> [*]');
  });

  it('applies an active class only when a valid state is given', () => {
    expect(toMermaid(SPEC, 'Funded')).toContain('class Funded active');
    expect(toMermaid(SPEC, 'Nonexistent')).not.toContain('class ');
    expect(toMermaid(SPEC)).not.toContain('class ');
  });
});

describe('layoutFlowchart', () => {
  it('places every state exactly once', () => {
    const layout = layoutFlowchart(SPEC);
    expect(layout.nodes.map((n) => n.id)).toEqual(['Draft', 'Funded', 'Released']);
  });

  it('orders states left to right by distance from the initial state', () => {
    const layout = layoutFlowchart(SPEC);
    const [draft, funded, released] = layout.nodes;
    expect(draft.x).toBeLessThan(funded.x);
    expect(funded.x).toBeLessThan(released.x);
  });

  it('flags the initial and terminal states', () => {
    const layout = layoutFlowchart(SPEC);
    expect(layout.nodes.find((n) => n.id === 'Draft')?.isInitial).toBe(true);
    expect(layout.nodes.find((n) => n.id === 'Released')?.isTerminal).toBe(true);
  });

  it('routes a self-transition as a loop', () => {
    const layout = layoutFlowchart(SPEC);
    const loop = layout.edges.find((e) => e.from === 'Funded' && e.to === 'Funded');
    expect(loop?.selfLoop).toBe(true);
  });

  it('enables only the edges leaving the active state', () => {
    const layout = layoutFlowchart(SPEC, 'Funded');
    const fund = layout.edges.find((e) => e.label.startsWith('fund'));
    const release = layout.edges.find((e) => e.label.startsWith('release'));
    expect(fund?.enabled).toBe(false);
    expect(release?.enabled).toBe(true);
  });

  it('still places states unreachable from the initial state', () => {
    const orphaned: ContractFlowchartSpec = {
      ...SPEC,
      states: [...SPEC.states, 'Archived'],
    };
    expect(layoutFlowchart(orphaned).nodes.map((n) => n.id)).toContain('Archived');
  });

  it('drops edges that reference a state the spec does not declare', () => {
    const dangling: ContractFlowchartSpec = {
      ...SPEC,
      transitions: [...SPEC.transitions, { name: 'ghost', from: 'Draft', to: 'Missing' }],
    };
    expect(layoutFlowchart(dangling).edges.some((e) => e.label === 'ghost')).toBe(false);
  });
});

describe('toSvg', () => {
  it('generates a well-formed svg root sized to the layout', () => {
    const svg = toSvg(SPEC);
    expect(svg.startsWith('<svg xmlns="http://www.w3.org/2000/svg"')).toBe(true);
    expect(svg.trimEnd().endsWith('</svg>')).toBe(true);
    expect(svg).toContain('viewBox="0 0 ');
  });

  it('renders a group per state and a path per transition', () => {
    const svg = toSvg(SPEC);
    SPEC.states.forEach((state) => {
      expect(svg).toContain(`data-state="${state}"`);
    });
    // Counted via data-edge so the two arrow-marker paths in <defs> are excluded.
    expect(svg.match(/data-edge="/g)).toHaveLength(SPEC.transitions.length);
  });

  it('marks the active state and no other', () => {
    const svg = toSvg(SPEC, 'Funded');
    expect(svg).toContain('data-state="Funded" data-active="true"');
    expect(svg.match(/data-active="true"/g)).toHaveLength(1);
  });

  it('defines arrow markers for both enabled and disabled edges', () => {
    const svg = toSvg(SPEC, 'Funded');
    expect(svg).toContain('<marker id="arrow"');
    expect(svg).toContain('<marker id="arrow-active"');
    expect(svg).toContain('marker-end="url(#arrow-active)"');
  });

  it('escapes state names so they cannot inject markup', () => {
    const hostile: ContractFlowchartSpec = {
      name: 'X',
      initial: '<script>',
      states: ['<script>'],
      transitions: [],
    };
    const svg = toSvg(hostile);
    expect(svg).not.toContain('<script>');
    expect(svg).toContain('&lt;script&gt;');
  });
});

describe('permittedTransitions', () => {
  it('returns only transitions leaving the given state', () => {
    expect(permittedTransitions(SPEC, 'Funded').map((t) => t.name)).toEqual(['top_up', 'release']);
  });

  it('returns nothing for a terminal state', () => {
    expect(permittedTransitions(SPEC, 'Released')).toEqual([]);
  });
});

describe('inferFlowchart', () => {
  it('advances a stage for each recognised lifecycle verb', () => {
    const spec = inferFlowchart('Vault', [
      { name: 'initialize' },
      { name: 'deposit' },
      { name: 'release' },
    ]);
    expect(spec.states).toEqual(['Uninitialized', 'Initialized', 'Funded', 'Settled']);
    expect(spec.transitions).toContainEqual({ name: 'deposit', from: 'Initialized', to: 'Funded' });
  });

  it('treats unrecognised functions as self-transitions on the current stage', () => {
    const spec = inferFlowchart('Vault', [{ name: 'initialize' }, { name: 'balance' }]);
    expect(spec.transitions).toContainEqual({
      name: 'balance',
      from: 'Initialized',
      to: 'Initialized',
    });
  });

  it('does not treat cancellation as the baseline for later stages', () => {
    const spec = inferFlowchart('Vault', [
      { name: 'initialize' },
      { name: 'cancel' },
      { name: 'deposit' },
    ]);
    expect(spec.transitions).toContainEqual({ name: 'deposit', from: 'Initialized', to: 'Funded' });
  });

  it('marks settlement and cancellation states as terminal', () => {
    const spec = inferFlowchart('Vault', [{ name: 'initialize' }, { name: 'release' }, { name: 'cancel' }]);
    expect(spec.terminal).toEqual(expect.arrayContaining(['Settled', 'Cancelled']));
  });

  it('produces a spec that renders', () => {
    const spec = inferFlowchart('Counter', [{ name: 'increment' }, { name: 'get_value' }]);
    expect(() => toSvg(spec)).not.toThrow();
    expect(toMermaid(spec)).toContain('stateDiagram-v2');
  });
});
