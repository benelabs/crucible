import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const toBlob = vi.fn(async () => new Blob(['%PDF-1.7'], { type: 'application/pdf' }));

// @react-pdf/renderer targets a real browser/node PDF pipeline, so the primitives
// are stubbed to plain elements and only the render call is asserted.
vi.mock('@react-pdf/renderer', () => ({
  Document: ({ children }: any) => <div data-testid="pdf-document">{children}</div>,
  Page: ({ children }: any) => <div>{children}</div>,
  Text: ({ children }: any) => <span>{children}</span>,
  View: ({ children }: any) => <div>{children}</div>,
  StyleSheet: { create: (styles: any) => styles },
  pdf: vi.fn(() => ({ toBlob })),
}));

import { pdf } from '@react-pdf/renderer';
import {
  AuditReportExport,
  MIN_BRANCH_COVERAGE,
  MIN_LINE_COVERAGE,
  SAMPLE_REPORT_INPUT,
  buildAuditReport,
  buildExecutiveSummary,
  buildSecurityChecklist,
  computeRiskScore,
  countBySeverity,
  formatDate,
  formatDuration,
  formatGas,
  formatMarkdownReport,
  formatPercent,
  openFindings,
  renderReportPdf,
  reportFileName,
  riskRating,
  type AuditReportInput,
  type StaticAnalysisFinding,
} from './AuditReportExport';

const finding = (overrides: Partial<StaticAnalysisFinding> = {}): StaticAnalysisFinding => ({
  id: 'CRU-000',
  title: 'Example finding',
  severity: 'low',
  location: 'src/lib.rs:1',
  description: 'Example description.',
  status: 'open',
  ...overrides,
});

const cleanInput = (overrides: Partial<AuditReportInput> = {}): AuditReportInput => ({
  contractName: 'Token Vault',
  contractId: 'CDLZ...HGCYSC',
  network: 'testnet',
  auditor: 'Crucible Static Analyzer',
  generatedAt: '2026-08-29T12:00:00.000Z',
  findings: [],
  coverage: { lines: 95, branches: 90, functions: 98 },
  tests: { total: 20, passed: 20, failed: 0, skipped: 0, durationMs: 1500 },
  gas: [{ functionName: 'deposit', averageGas: 14_500, maxGas: 21_300, invocations: 1280 }],
  dependencies: [{ name: 'soroban-sdk', version: '25.0.0', score: 96, advisories: 0 }],
  ...overrides,
});

describe('formatters', () => {
  it('formats percentages to one decimal', () => {
    expect(formatPercent(87.42)).toBe('87.4%');
    expect(formatPercent(100)).toBe('100.0%');
  });

  it('formats gas with grouping and a unit', () => {
    expect(formatGas(14_500)).toBe('14,500 stroops');
    expect(formatGas(14_500.6)).toBe('14,501 stroops');
  });

  it('formats sub-second durations in ms and longer ones in seconds', () => {
    expect(formatDuration(820)).toBe('820 ms');
    expect(formatDuration(4820)).toBe('4.82 s');
  });

  it('formats ISO timestamps and passes through unparseable input', () => {
    expect(formatDate('2026-08-29T12:00:00.000Z')).toBe('2026-08-29 12:00:00');
    expect(formatDate('not-a-date')).toBe('not-a-date');
  });
});

describe('countBySeverity', () => {
  it('counts every severity bucket, including empty ones', () => {
    const counts = countBySeverity([
      finding({ severity: 'critical' }),
      finding({ severity: 'high' }),
      finding({ severity: 'high' }),
    ]);

    expect(counts).toEqual({ critical: 1, high: 2, medium: 0, low: 0, info: 0 });
  });
});

describe('openFindings', () => {
  it('keeps only findings still open', () => {
    const findings = [
      finding({ id: 'a', status: 'open' }),
      finding({ id: 'b', status: 'resolved' }),
      finding({ id: 'c', status: 'accepted' }),
    ];

    expect(openFindings(findings).map((item) => item.id)).toEqual(['a']);
  });
});

describe('computeRiskScore', () => {
  it('is 100 for a clean audit', () => {
    expect(computeRiskScore(cleanInput())).toBe(100);
  });

  it('ignores findings that are no longer open', () => {
    expect(
      computeRiskScore(cleanInput({ findings: [finding({ severity: 'critical', status: 'resolved' })] })),
    ).toBe(100);
  });

  it('penalises open findings by severity', () => {
    expect(computeRiskScore(cleanInput({ findings: [finding({ severity: 'critical' })] }))).toBe(70);
    expect(computeRiskScore(cleanInput({ findings: [finding({ severity: 'high' })] }))).toBe(85);
    expect(computeRiskScore(cleanInput({ findings: [finding({ severity: 'info' })] }))).toBe(100);
  });

  it('penalises coverage shortfall, advisories and failing tests', () => {
    expect(computeRiskScore(cleanInput({ coverage: { lines: 80, branches: 90, functions: 98 } }))).toBe(95);
    expect(
      computeRiskScore(
        cleanInput({ dependencies: [{ name: 'bad', version: '0.1.0', score: 30, advisories: 2 }] }),
      ),
    ).toBe(90);
    expect(
      computeRiskScore(cleanInput({ tests: { total: 20, passed: 19, failed: 1, skipped: 0, durationMs: 10 } })),
    ).toBe(90);
  });

  it('clamps to the 0-100 range', () => {
    const findings = Array.from({ length: 6 }, (_, index) =>
      finding({ id: `CRU-${index}`, severity: 'critical' }),
    );

    expect(computeRiskScore(cleanInput({ findings }))).toBe(0);
  });
});

describe('riskRating', () => {
  it('maps scores onto rating bands', () => {
    expect(riskRating(100)).toBe('Low');
    expect(riskRating(90)).toBe('Low');
    expect(riskRating(89)).toBe('Moderate');
    expect(riskRating(70)).toBe('Moderate');
    expect(riskRating(69)).toBe('Elevated');
    expect(riskRating(40)).toBe('Elevated');
    expect(riskRating(39)).toBe('Critical');
  });
});

describe('buildSecurityChecklist', () => {
  it('passes every gate for a clean audit', () => {
    const checklist = buildSecurityChecklist(cleanInput());

    expect(checklist.every((item) => item.passed)).toBe(true);
    expect(checklist.map((item) => item.id)).toEqual([
      'no-critical',
      'no-high',
      'line-coverage',
      'branch-coverage',
      'tests-passing',
      'dependency-advisories',
    ]);
  });

  it('fails the coverage gates just below the thresholds', () => {
    const checklist = buildSecurityChecklist(
      cleanInput({
        coverage: { lines: MIN_LINE_COVERAGE - 0.1, branches: MIN_BRANCH_COVERAGE - 0.1, functions: 98 },
      }),
    );

    expect(checklist.find((item) => item.id === 'line-coverage')).toMatchObject({
      passed: false,
      detail: formatPercent(MIN_LINE_COVERAGE - 0.1),
    });
    expect(checklist.find((item) => item.id === 'branch-coverage')?.passed).toBe(false);
  });

  it('reports open critical findings and dependency advisories in the detail text', () => {
    const checklist = buildSecurityChecklist(
      cleanInput({
        findings: [finding({ severity: 'critical' }), finding({ id: 'x', severity: 'high' })],
        dependencies: [{ name: 'bad', version: '0.4.2', score: 41, advisories: 2 }],
      }),
    );

    expect(checklist.find((item) => item.id === 'no-critical')).toMatchObject({
      passed: false,
      detail: '1 open critical finding(s)',
    });
    expect(checklist.find((item) => item.id === 'dependency-advisories')).toMatchObject({
      passed: false,
      detail: '2 advisory/advisories across 1 package(s)',
    });
  });

  it('fails the test gate when no tests ran', () => {
    const checklist = buildSecurityChecklist(
      cleanInput({ tests: { total: 0, passed: 0, failed: 0, skipped: 0, durationMs: 0 } }),
    );

    expect(checklist.find((item) => item.id === 'tests-passing')?.passed).toBe(false);
  });
});

describe('buildExecutiveSummary', () => {
  it('summarises open findings with a severity breakdown', () => {
    const input = cleanInput({
      findings: [finding({ severity: 'high' }), finding({ id: 'x', severity: 'medium' })],
    });

    const summary = buildExecutiveSummary(input, 78, 'Moderate');

    expect(summary).toContain('Token Vault was analysed on testnet by Crucible Static Analyzer.');
    expect(summary).toContain('2 static-analysis finding(s), of which 2 remain open (1 high, 1 medium)');
    expect(summary).toContain('95.0% of lines with 20/20 tests passing');
    expect(summary).toContain('78/100, rated Moderate');
  });

  it('omits the breakdown when nothing is open', () => {
    expect(buildExecutiveSummary(cleanInput(), 100, 'Low')).toContain('0 remain open.');
  });
});

describe('buildAuditReport', () => {
  it('derives counts, score, rating, checklist and summary from the input', () => {
    const report = buildAuditReport(SAMPLE_REPORT_INPUT);

    expect(report.severityCounts).toEqual({ critical: 1, high: 1, medium: 1, low: 0, info: 0 });
    expect(report.openFindings).toBe(2);
    // 100 - 15 (open high) - 7 (open medium) - 1.3 (coverage) - 10 (advisories) - 10 (failing test)
    expect(report.riskScore).toBe(57);
    expect(report.riskRating).toBe('Elevated');
    expect(report.checklist).toHaveLength(6);
    expect(report.contractName).toBe(SAMPLE_REPORT_INPUT.contractName);
  });
});

describe('formatMarkdownReport', () => {
  const markdown = formatMarkdownReport(buildAuditReport(SAMPLE_REPORT_INPUT));

  it('opens with a titled heading and a metadata table', () => {
    expect(markdown.startsWith('# Contract Audit Report — Escrow Vault\n')).toBe(true);
    expect(markdown).toContain('| Field | Value |');
    expect(markdown).toContain('| Security score | 57/100 (Elevated) |');
    expect(markdown).toContain('| Generated | 2026-08-29 12:00:00 |');
  });

  it('includes every required section', () => {
    for (const heading of [
      '## Executive Summary',
      '## Security Checklist',
      '## Static Analysis Findings',
      '## Test Results',
      '## Gas Expenditure',
      '## Dependency Security',
    ]) {
      expect(markdown).toContain(heading);
    }
  });

  it('renders the checklist as GitHub task list items', () => {
    expect(markdown).toContain('- [x] No open critical findings — 0 open critical finding(s)');
    expect(markdown).toContain('- [ ] No open high-severity findings — 1 open high-severity finding(s)');
  });

  it('renders findings, gas and dependency rows as tables', () => {
    expect(markdown).toContain(
      '| HIGH | CRU-001 | Unchecked arithmetic in fee calculation | src/lib.rs:142 | open |',
    );
    expect(markdown).toContain('| deposit | 14,500 stroops | 21,300 stroops | 1,280 |');
    expect(markdown).toContain('| vulnerable-crate | 0.4.2 | 41/100 | 2 |');
    expect(markdown).toContain('| Duration | 4.82 s |');
  });

  it('states explicitly when there are no findings', () => {
    const clean = formatMarkdownReport(buildAuditReport(cleanInput()));

    expect(clean).toContain('## Static Analysis Findings\n\nNo findings were reported.');
  });

  it('ends with a single trailing newline', () => {
    expect(markdown.endsWith('\n')).toBe(true);
    expect(markdown.endsWith('\n\n')).toBe(false);
  });
});

describe('reportFileName', () => {
  it('slugifies the contract name per extension', () => {
    const report = buildAuditReport(SAMPLE_REPORT_INPUT);

    expect(reportFileName(report, 'md')).toBe('escrow-vault-audit-report.md');
    expect(reportFileName(report, 'pdf')).toBe('escrow-vault-audit-report.pdf');
  });

  it('falls back when the name has no usable characters', () => {
    const report = buildAuditReport(cleanInput({ contractName: '***' }));

    expect(reportFileName(report, 'md')).toBe('contract-audit-report.md');
  });
});

describe('renderReportPdf', () => {
  it('renders the audit document to a blob', async () => {
    const blob = await renderReportPdf(buildAuditReport(cleanInput()));

    expect(pdf).toHaveBeenCalled();
    expect(toBlob).toHaveBeenCalled();
    expect(blob.type).toBe('application/pdf');
  });
});

describe('AuditReportExport', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    URL.createObjectURL = vi.fn(() => 'blob:audit');
    URL.revokeObjectURL = vi.fn();
  });

  it('renders the score, summary and checklist', () => {
    render(<AuditReportExport />);

    expect(screen.getByTestId('risk-score')).toHaveTextContent('57/100');
    expect(screen.getByTestId('risk-rating')).toHaveTextContent('Elevated');
    expect(screen.getByTestId('executive-summary')).toHaveTextContent('Escrow Vault was analysed on testnet');
    expect(screen.getByTestId('checklist-no-critical')).toHaveTextContent('PASS');
    expect(screen.getByTestId('checklist-no-high')).toHaveTextContent('FAIL');
  });

  it('shows the markdown preview', () => {
    render(<AuditReportExport input={cleanInput()} />);

    expect(screen.getByTestId('markdown-preview')).toHaveTextContent('Contract Audit Report');
  });

  it('exports a markdown file', () => {
    render(<AuditReportExport />);

    fireEvent.click(screen.getByTestId('export-markdown'));

    expect(URL.createObjectURL).toHaveBeenCalled();
    expect(screen.getByTestId('export-status')).toHaveTextContent(
      'Markdown report exported as escrow-vault-audit-report.md',
    );
  });

  it('exports a PDF file', async () => {
    render(<AuditReportExport />);

    fireEvent.click(screen.getByTestId('export-pdf'));

    await waitFor(() => {
      expect(screen.getByTestId('export-status')).toHaveTextContent(
        'PDF report exported as escrow-vault-audit-report.pdf',
      );
    });
    expect(pdf).toHaveBeenCalled();
  });

  it('reports a failed PDF render', async () => {
    toBlob.mockRejectedValueOnce(new Error('font missing'));
    render(<AuditReportExport />);

    fireEvent.click(screen.getByTestId('export-pdf'));

    await waitFor(() => {
      expect(screen.getByTestId('export-status')).toHaveTextContent('PDF export failed: font missing');
    });
  });
});
