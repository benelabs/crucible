import React, { useCallback, useEffect, useRef, useState } from 'react';
import Editor, { type Monaco, type OnMount } from '@monaco-editor/react';
import type { editor as MonacoEditor, IDisposable, Position } from 'monaco-editor';
import { AlertTriangle, Code2, Info, XCircle } from 'lucide-react';
import './ContractSourceEditor.css';

export type DiagnosticSeverity = 'error' | 'warning' | 'info' | 'hint';

/** A compiler message normalised to 1-based line/column, as rustc reports them. */
export interface CompilerDiagnostic {
  severity: DiagnosticSeverity;
  message: string;
  line: number;
  column: number;
  endLine?: number;
  endColumn?: number;
  code?: string;
}

export interface MonacoMarker {
  severity: number;
  message: string;
  startLineNumber: number;
  startColumn: number;
  endLineNumber: number;
  endColumn: number;
  code?: string;
  source: string;
}

/** Mirrors monaco.MarkerSeverity, which is not available until monaco loads. */
export const MARKER_SEVERITY: Record<DiagnosticSeverity, number> = {
  hint: 1,
  info: 2,
  warning: 4,
  error: 8,
};

/** Identifies markers owned by this editor so we only ever clear our own. */
export const MARKER_OWNER = 'crucible-compiler';

export const DEFAULT_RUST_SOURCE = `#![no_std]
use soroban_sdk::{contract, contractimpl, Env, Symbol, symbol_short};

const COUNTER: Symbol = symbol_short!("COUNTER");

#[contract]
pub struct IncrementContract;

#[contractimpl]
impl IncrementContract {
    /// Increments the stored counter and returns the new value.
    pub fn increment(env: Env) -> u32 {
        let mut count: u32 = env.storage().instance().get(&COUNTER).unwrap_or(0);
        count += 1;
        env.storage().instance().set(&COUNTER, &count);
        env.storage().instance().extend_ttl(50, 100);
        count
    }
}
`;

/**
 * Converts compiler diagnostics into monaco markers. Diagnostics without an
 * explicit end default to highlighting to the end of the start line.
 */
export function toMonacoMarkers(diagnostics: CompilerDiagnostic[]): MonacoMarker[] {
  return diagnostics.map((diagnostic) => ({
    severity: MARKER_SEVERITY[diagnostic.severity],
    message: diagnostic.code ? `[${diagnostic.code}] ${diagnostic.message}` : diagnostic.message,
    startLineNumber: diagnostic.line,
    startColumn: diagnostic.column,
    endLineNumber: diagnostic.endLine ?? diagnostic.line,
    // Number.MAX_SAFE_INTEGER makes monaco clamp to the end of the line.
    endColumn: diagnostic.endColumn ?? Number.MAX_SAFE_INTEGER,
    code: diagnostic.code,
    source: MARKER_OWNER,
  }));
}

const RUSTC_HEADER = /^(error|warning)(?:\[([A-Z]\d+)\])?:\s*(.+)$/;
const RUSTC_LOCATION = /^\s*-->\s*(.+?):(\d+):(\d+)\s*$/;

/**
 * Extracts diagnostics from raw `cargo build` output. A message is only
 * emitted once its `-->` location line has been seen.
 */
export function parseRustcDiagnostics(log: string): CompilerDiagnostic[] {
  const diagnostics: CompilerDiagnostic[] = [];
  let pending: { severity: DiagnosticSeverity; message: string; code?: string } | null = null;

  for (const rawLine of log.split('\n')) {
    const header = RUSTC_HEADER.exec(rawLine.trim());
    if (header) {
      pending = {
        severity: header[1] === 'error' ? 'error' : 'warning',
        code: header[2],
        message: header[3],
      };
      continue;
    }

    const location = RUSTC_LOCATION.exec(rawLine);
    if (location && pending) {
      diagnostics.push({
        ...pending,
        line: Number(location[2]),
        column: Number(location[3]),
      });
      pending = null;
    }
  }

  return diagnostics;
}

export interface RustCompletion {
  label: string;
  insertText: string;
  detail: string;
}

/** Soroban-flavoured completions offered alongside monaco's Rust tokenizer. */
export const RUST_COMPLETIONS: RustCompletion[] = [
  { label: 'contract', insertText: '#[contract]\npub struct ${1:MyContract};', detail: 'Soroban contract type' },
  { label: 'contractimpl', insertText: '#[contractimpl]\nimpl ${1:MyContract} {\n\t$0\n}', detail: 'Soroban contract impl block' },
  { label: 'contracttype', insertText: '#[contracttype]\npub struct ${1:MyType} {\n\t$0\n}', detail: 'Soroban serialisable type' },
  { label: 'symbol_short', insertText: 'symbol_short!("${1:KEY}")', detail: 'Short symbol literal (max 9 chars)' },
  { label: 'storage_instance_get', insertText: 'env.storage().instance().get(&${1:KEY}).unwrap_or(${2:0})', detail: 'Read instance storage' },
  { label: 'storage_instance_set', insertText: 'env.storage().instance().set(&${1:KEY}, &${2:value});', detail: 'Write instance storage' },
  { label: 'extend_ttl', insertText: 'env.storage().instance().extend_ttl(${1:50}, ${2:100});', detail: 'Extend instance storage TTL' },
  { label: 'require_auth', insertText: '${1:address}.require_auth();', detail: 'Assert caller authorisation' },
  { label: 'panic_with_error', insertText: 'panic_with_error!(&env, ${1:Error::NotFound});', detail: 'Abort with a contract error' },
];

/**
 * Registers the Soroban completion provider. Returns monaco's disposable so
 * the provider is torn down with the component rather than leaking per mount.
 */
export function registerRustCompletions(monaco: Monaco): IDisposable {
  return monaco.languages.registerCompletionItemProvider('rust', {
    provideCompletionItems: (model: MonacoEditor.ITextModel, position: Position) => {
      const word = model.getWordUntilPosition(position);
      const range = {
        startLineNumber: position.lineNumber,
        endLineNumber: position.lineNumber,
        startColumn: word.startColumn,
        endColumn: word.endColumn,
      };

      return {
        suggestions: RUST_COMPLETIONS.map((completion) => ({
          label: completion.label,
          kind: monaco.languages.CompletionItemKind.Snippet,
          insertText: completion.insertText,
          insertTextRules: monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
          detail: completion.detail,
          range,
        })),
      };
    },
  });
}

export function countDiagnostics(diagnostics: CompilerDiagnostic[]): Record<DiagnosticSeverity, number> {
  const counts: Record<DiagnosticSeverity, number> = { error: 0, warning: 0, info: 0, hint: 0 };
  for (const diagnostic of diagnostics) {
    counts[diagnostic.severity] += 1;
  }
  return counts;
}

export interface ContractSourceEditorProps {
  value?: string;
  defaultValue?: string;
  onChange?: (value: string) => void;
  diagnostics?: CompilerDiagnostic[];
  /** Monaco theme id; defaults to the dark theme matching the app shell. */
  theme?: string;
  height?: string | number;
  readOnly?: boolean;
}

export const ContractSourceEditor: React.FC<ContractSourceEditorProps> = ({
  value,
  defaultValue = DEFAULT_RUST_SOURCE,
  onChange,
  diagnostics = [],
  theme = 'vs-dark',
  height = 460,
  readOnly = false,
}) => {
  const isControlled = value !== undefined;
  const [internalValue, setInternalValue] = useState(defaultValue);
  const source = isControlled ? value : internalValue;

  const editorRef = useRef<MonacoEditor.IStandaloneCodeEditor | null>(null);
  const monacoRef = useRef<Monaco | null>(null);
  const completionRef = useRef<IDisposable | null>(null);
  const [ready, setReady] = useState(false);

  const handleMount = useCallback<OnMount>((editor, monaco) => {
    editorRef.current = editor;
    monacoRef.current = monaco;
    completionRef.current = registerRustCompletions(monaco);
    setReady(true);
  }, []);

  useEffect(() => () => completionRef.current?.dispose(), []);

  // Push markers into the gutter whenever the diagnostics change.
  useEffect(() => {
    const editor = editorRef.current;
    const monaco = monacoRef.current;
    const model = editor?.getModel();
    if (!monaco || !model) return;

    monaco.editor.setModelMarkers(model, MARKER_OWNER, toMonacoMarkers(diagnostics));
  }, [diagnostics, ready]);

  const handleChange = useCallback(
    (next: string | undefined) => {
      const text = next ?? '';
      if (!isControlled) setInternalValue(text);
      onChange?.(text);
    },
    [isControlled, onChange],
  );

  const counts = countDiagnostics(diagnostics);
  const lineCount = source.split('\n').length;

  return (
    <div className="source-editor-container" data-testid="contract-source-editor">
      <div className="source-editor-header">
        <div className="source-editor-icon-wrapper">
          <Code2 className="source-editor-icon" />
        </div>
        <div>
          <h2>Contract Source Editor</h2>
          <p>Rust syntax highlighting, folding and inline compiler diagnostics</p>
        </div>
      </div>

      <div className="source-editor-statusbar">
        <span className="source-editor-badge" data-testid="editor-lines">
          {lineCount} lines
        </span>
        <span className="source-editor-badge source-editor-badge--error" data-testid="count-error">
          <XCircle size={12} />
          {counts.error} errors
        </span>
        <span className="source-editor-badge source-editor-badge--warning" data-testid="count-warning">
          <AlertTriangle size={12} />
          {counts.warning} warnings
        </span>
        <span className="source-editor-badge" data-testid="count-info">
          <Info size={12} />
          {counts.info + counts.hint} notes
        </span>
      </div>

      <div className="source-editor-surface">
        <Editor
          height={height}
          defaultLanguage="rust"
          language="rust"
          theme={theme}
          value={source}
          onChange={handleChange}
          onMount={handleMount}
          options={{
            readOnly,
            fontSize: 13,
            fontFamily: "'JetBrains Mono', ui-monospace, monospace",
            minimap: { enabled: false },
            folding: true,
            foldingStrategy: 'indentation',
            lineNumbers: 'on',
            glyphMargin: true,
            renderLineHighlight: 'line',
            scrollBeyondLastLine: false,
            smoothScrolling: true,
            tabSize: 4,
            automaticLayout: true,
            quickSuggestions: true,
          }}
        />
      </div>

      {diagnostics.length > 0 && (
        <ul className="source-editor-diagnostics" data-testid="diagnostic-list">
          {diagnostics.map((diagnostic, index) => (
            <li
              className={`source-editor-diagnostic source-editor-diagnostic--${diagnostic.severity}`}
              key={`${diagnostic.line}-${diagnostic.column}-${index}`}
              data-testid={`diagnostic-${index}`}
            >
              <span className="source-editor-diagnostic-location">
                {diagnostic.line}:{diagnostic.column}
              </span>
              <span>{diagnostic.code ? `[${diagnostic.code}] ` : ''}{diagnostic.message}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
};
