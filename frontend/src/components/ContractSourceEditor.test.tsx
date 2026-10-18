import { render, screen, fireEvent } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

const setModelMarkers = vi.fn();
const getModel = vi.fn(() => ({ uri: 'inmemory://contract.rs' }));
const disposeCompletions = vi.fn();
const registerCompletionItemProvider = vi.fn(() => ({ dispose: disposeCompletions }));

/** Captures the provider monaco was handed, so it can be exercised directly. */
interface CapturedProvider {
  provideCompletionItems: (
    model: { getWordUntilPosition: () => { startColumn: number; endColumn: number } },
    position: { lineNumber: number },
  ) => { suggestions: Record<string, unknown>[] };
}
let capturedProvider: CapturedProvider | null = null;

const monacoStub = {
  editor: { setModelMarkers },
  languages: {
    registerCompletionItemProvider: (language: string, provider: CapturedProvider) => {
      capturedProvider = provider;
      return registerCompletionItemProvider(language, provider);
    },
    CompletionItemKind: { Snippet: 27 },
    CompletionItemInsertTextRule: { InsertAsSnippet: 4 },
  },
};

interface EditorStubProps {
  value?: string;
  onChange?: (value: string) => void;
  onMount?: (editor: { getModel: typeof getModel }, monaco: typeof monacoStub) => void;
  options?: { readOnly?: boolean; folding?: boolean };
}

// Monaco needs a real browser: the editor is stubbed with a textarea so content
// synchronisation can still be asserted end to end.
vi.mock('@monaco-editor/react', () => ({
  default: ({ value, onChange, onMount, options }: EditorStubProps) => {
    return (
      <textarea
        data-testid="monaco-textarea"
        data-read-only={String(Boolean(options?.readOnly))}
        data-folding={String(Boolean(options?.folding))}
        value={value}
        onChange={(event) => onChange?.(event.target.value)}
        ref={() => {
          if (!onMount) return;
          onMount({ getModel }, monacoStub);
        }}
      />
    );
  },
}));

import {
  ContractSourceEditor,
  DEFAULT_RUST_SOURCE,
  MARKER_OWNER,
  MARKER_SEVERITY,
  RUST_COMPLETIONS,
  countDiagnostics,
  parseRustcDiagnostics,
  registerRustCompletions,
  toMonacoMarkers,
  type CompilerDiagnostic,
} from './ContractSourceEditor';

const diagnostic = (overrides: Partial<CompilerDiagnostic> = {}): CompilerDiagnostic => ({
  severity: 'error',
  message: 'cannot find value `count` in this scope',
  line: 12,
  column: 5,
  ...overrides,
});

describe('toMonacoMarkers', () => {
  it('maps severities onto monaco marker severities', () => {
    const markers = toMonacoMarkers([
      diagnostic({ severity: 'error' }),
      diagnostic({ severity: 'warning' }),
      diagnostic({ severity: 'info' }),
      diagnostic({ severity: 'hint' }),
    ]);

    expect(markers.map((marker) => marker.severity)).toEqual([
      MARKER_SEVERITY.error,
      MARKER_SEVERITY.warning,
      MARKER_SEVERITY.info,
      MARKER_SEVERITY.hint,
    ]);
  });

  it('anchors the marker at the reported 1-based position', () => {
    const [marker] = toMonacoMarkers([diagnostic({ line: 12, column: 5 })]);

    expect(marker).toMatchObject({
      startLineNumber: 12,
      startColumn: 5,
      endLineNumber: 12,
      source: MARKER_OWNER,
    });
  });

  it('extends to the end of the line when no end position is given', () => {
    const [marker] = toMonacoMarkers([diagnostic()]);

    expect(marker.endColumn).toBe(Number.MAX_SAFE_INTEGER);
  });

  it('honours an explicit end position', () => {
    const [marker] = toMonacoMarkers([diagnostic({ endLine: 14, endColumn: 20 })]);

    expect(marker).toMatchObject({ endLineNumber: 14, endColumn: 20 });
  });

  it('prefixes the rustc error code into the hover message', () => {
    const [marker] = toMonacoMarkers([diagnostic({ code: 'E0425' })]);

    expect(marker.message).toBe('[E0425] cannot find value `count` in this scope');
    expect(marker.code).toBe('E0425');
  });

  it('returns nothing for no diagnostics', () => {
    expect(toMonacoMarkers([])).toEqual([]);
  });
});

describe('parseRustcDiagnostics', () => {
  it('parses an error with its code and location', () => {
    const log = [
      'error[E0425]: cannot find value `count` in this scope',
      '  --> src/lib.rs:12:5',
      '   |',
      '12 |     count += 1;',
      '   |     ^^^^^ not found in this scope',
    ].join('\n');

    expect(parseRustcDiagnostics(log)).toEqual([
      {
        severity: 'error',
        code: 'E0425',
        message: 'cannot find value `count` in this scope',
        line: 12,
        column: 5,
      },
    ]);
  });

  it('parses warnings without a code', () => {
    const log = ['warning: unused variable: `env`', ' --> src/lib.rs:7:18'].join('\n');

    expect(parseRustcDiagnostics(log)).toEqual([
      { severity: 'warning', code: undefined, message: 'unused variable: `env`', line: 7, column: 18 },
    ]);
  });

  it('parses several diagnostics from one build log', () => {
    const log = [
      'error[E0433]: failed to resolve: use of undeclared crate',
      '  --> src/lib.rs:3:5',
      'warning: unused import: `Symbol`',
      '  --> src/lib.rs:2:24',
    ].join('\n');

    const parsed = parseRustcDiagnostics(log);

    expect(parsed).toHaveLength(2);
    expect(parsed.map((item) => item.severity)).toEqual(['error', 'warning']);
    expect(parsed[1].line).toBe(2);
  });

  it('ignores a message that never reports a location', () => {
    expect(parseRustcDiagnostics('error: could not compile `contract`')).toEqual([]);
  });

  it('returns nothing for a successful build log', () => {
    expect(parseRustcDiagnostics('Finished release [optimized] target(s) in 4.21s')).toEqual([]);
  });
});

describe('countDiagnostics', () => {
  it('counts each severity bucket', () => {
    const counts = countDiagnostics([
      diagnostic({ severity: 'error' }),
      diagnostic({ severity: 'error' }),
      diagnostic({ severity: 'warning' }),
    ]);

    expect(counts).toEqual({ error: 2, warning: 1, info: 0, hint: 0 });
  });
});

describe('registerRustCompletions', () => {
  it('registers Soroban snippets against the rust language', () => {
    registerRustCompletions(monacoStub as never);

    expect(registerCompletionItemProvider).toHaveBeenCalledWith('rust', expect.anything());

    const model = { getWordUntilPosition: () => ({ startColumn: 3, endColumn: 7 }) };
    const { suggestions } = capturedProvider!.provideCompletionItems(model, { lineNumber: 4 });

    expect(suggestions).toHaveLength(RUST_COMPLETIONS.length);
    expect(suggestions[0]).toMatchObject({
      label: 'contract',
      kind: 27,
      insertTextRules: 4,
      range: { startLineNumber: 4, endLineNumber: 4, startColumn: 3, endColumn: 7 },
    });
  });
});

describe('ContractSourceEditor', () => {
  it('renders the default Soroban source', () => {
    render(<ContractSourceEditor />);

    expect(screen.getByTestId('monaco-textarea')).toHaveValue(DEFAULT_RUST_SOURCE);
    expect(screen.getByText('Contract Source Editor')).toBeInTheDocument();
  });

  it('enables folding and honours readOnly through editor options', () => {
    render(<ContractSourceEditor readOnly />);

    const textarea = screen.getByTestId('monaco-textarea');
    expect(textarea).toHaveAttribute('data-folding', 'true');
    expect(textarea).toHaveAttribute('data-read-only', 'true');
  });

  it('synchronises edits out through onChange when uncontrolled', () => {
    const onChange = vi.fn();
    render(<ContractSourceEditor defaultValue="fn main() {}" onChange={onChange} />);

    fireEvent.change(screen.getByTestId('monaco-textarea'), { target: { value: 'fn main() { let x = 1; }' } });

    expect(onChange).toHaveBeenCalledWith('fn main() { let x = 1; }');
    // Uncontrolled editors keep their own value in sync.
    expect(screen.getByTestId('monaco-textarea')).toHaveValue('fn main() { let x = 1; }');
    expect(screen.getByTestId('editor-lines')).toHaveTextContent('1 lines');
  });

  it('stays pinned to the value prop when controlled', () => {
    const onChange = vi.fn();
    const { rerender } = render(<ContractSourceEditor value="fn a() {}" onChange={onChange} />);

    fireEvent.change(screen.getByTestId('monaco-textarea'), { target: { value: 'fn b() {}' } });

    expect(onChange).toHaveBeenCalledWith('fn b() {}');
    // The parent owns the value, so nothing changes until it re-renders.
    expect(screen.getByTestId('monaco-textarea')).toHaveValue('fn a() {}');

    rerender(<ContractSourceEditor value="fn b() {}" onChange={onChange} />);
    expect(screen.getByTestId('monaco-textarea')).toHaveValue('fn b() {}');
  });

  it('treats a cleared editor as an empty string', () => {
    const onChange = vi.fn();
    render(<ContractSourceEditor defaultValue="fn main() {}" onChange={onChange} />);

    fireEvent.change(screen.getByTestId('monaco-textarea'), { target: { value: '' } });

    expect(onChange).toHaveBeenCalledWith('');
  });

  it('counts lines from the current content', () => {
    render(<ContractSourceEditor value={'a\nb\nc'} />);

    expect(screen.getByTestId('editor-lines')).toHaveTextContent('3 lines');
  });

  it('pushes diagnostics into the gutter as markers', () => {
    setModelMarkers.mockClear();
    render(<ContractSourceEditor value="fn main() {}" diagnostics={[diagnostic({ code: 'E0425' })]} />);

    expect(setModelMarkers).toHaveBeenCalledWith(
      expect.anything(),
      MARKER_OWNER,
      [expect.objectContaining({ startLineNumber: 12, severity: MARKER_SEVERITY.error })],
    );
  });

  it('clears markers when the diagnostics are resolved', () => {
    const { rerender } = render(
      <ContractSourceEditor value="fn main() {}" diagnostics={[diagnostic()]} />,
    );
    setModelMarkers.mockClear();

    rerender(<ContractSourceEditor value="fn main() {}" diagnostics={[]} />);

    expect(setModelMarkers).toHaveBeenCalledWith(expect.anything(), MARKER_OWNER, []);
  });

  it('lists the diagnostics under the editor and summarises the counts', () => {
    render(
      <ContractSourceEditor
        value="fn main() {}"
        diagnostics={[diagnostic({ code: 'E0425' }), diagnostic({ severity: 'warning', message: 'unused' })]}
      />,
    );

    expect(screen.getByTestId('count-error')).toHaveTextContent('1 errors');
    expect(screen.getByTestId('count-warning')).toHaveTextContent('1 warnings');
    expect(screen.getByTestId('diagnostic-0')).toHaveTextContent('12:5');
    expect(screen.getByTestId('diagnostic-0')).toHaveTextContent('[E0425]');
  });

  it('omits the diagnostics list when the build is clean', () => {
    render(<ContractSourceEditor value="fn main() {}" />);

    expect(screen.queryByTestId('diagnostic-list')).not.toBeInTheDocument();
  });
});
