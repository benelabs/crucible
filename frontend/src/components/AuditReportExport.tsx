import React, { useMemo, useState } from 'react';
import { Document, Page, StyleSheet, Text, View, pdf } from '@react-pdf/renderer';
import { FileText, Download, ShieldCheck } from 'lucide-react';
import './AuditReportExport.css';

export type FindingSeverity = 'critical' | 'high' | 'medium' | 'low' | 'info';

export type FindingStatus = 'open' | 'resolved' | 'accepted';

export interface StaticAnalysisFinding {
  id: string;
  title: string;
  severity: FindingSeverity;
  location: string;
  description: string;
  status: FindingStatus;
}

export interface CoverageMetrics {
  lines: number;
  branches: number;
  functions: number;
}

export interface TestResults {
  total: number;
  passed: number;
  failed: number;
  skipped: number;
  durationMs: number;
}

export interface GasExpenditure {
  functionName: string;
  averageGas: number;
  maxGas: number;
  invocations: number;
}

export interface DependencyScore {
  name: string;
  version: string;
  /** 0-100 security score reported by the dependency analyzer. */
  score: number;
  advisories: number;
}

export interface AuditReportInput {
  contractName: string;
  contractId: string;
  network: string;
  auditor: string;
  generatedAt: string;
  findings: StaticAnalysisFinding[];
  coverage: CoverageMetrics;
  tests: TestResults;
  gas: GasExpenditure[];
  dependencies: DependencyScore[];
}

export interface ChecklistItem {
  id: string;
  label: string;
  passed: boolean;
  detail: string;
}

export type RiskRating = 'Low' | 'Moderate' | 'Elevated' | 'Critical';

export interface AuditReport extends AuditReportInput {
  severityCounts: Record<FindingSeverity, number>;
  openFindings: number;
  riskScore: number;
  riskRating: RiskRating;
  checklist: ChecklistItem[];
  executiveSummary: string;
}

export const SEVERITY_ORDER: FindingSeverity[] = ['critical', 'high', 'medium', 'low', 'info'];

/** Risk-score penalty applied per open finding of each severity. */
const SEVERITY_WEIGHTS: Record<FindingSeverity, number> = {
  critical: 30,
  high: 15,
  medium: 7,
  low: 2,
  info: 0,
};

export const MIN_LINE_COVERAGE = 90;
export const MIN_BRANCH_COVERAGE = 80;

export function formatPercent(value: number): string {
  return `${value.toFixed(1)}%`;
}

export function formatGas(value: number): string {
  return `${Math.round(value).toLocaleString('en-US')} stroops`;
}

export function formatDuration(durationMs: number): string {
  if (durationMs < 1000) return `${Math.round(durationMs)} ms`;
  return `${(durationMs / 1000).toFixed(2)} s`;
}

export function formatDate(isoDate: string): string {
  const date = new Date(isoDate);
  return Number.isNaN(date.getTime()) ? isoDate : date.toISOString().replace('T', ' ').slice(0, 19);
}

export function countBySeverity(findings: StaticAnalysisFinding[]): Record<FindingSeverity, number> {
  const counts = { critical: 0, high: 0, medium: 0, low: 0, info: 0 };
  for (const finding of findings) {
    counts[finding.severity] += 1;
  }
  return counts;
}

export function openFindings(findings: StaticAnalysisFinding[]): StaticAnalysisFinding[] {
  return findings.filter((finding) => finding.status === 'open');
}

/**
 * A 0-100 posture score: open findings and dependency advisories subtract by
 * severity, and coverage below the gate subtracts its own shortfall.
 */
export function computeRiskScore(input: Pick<AuditReportInput, 'findings' | 'coverage' | 'dependencies' | 'tests'>): number {
  let score = 100;

  for (const finding of openFindings(input.findings)) {
    score -= SEVERITY_WEIGHTS[finding.severity];
  }

  score -= Math.max(0, MIN_LINE_COVERAGE - input.coverage.lines) / 2;
  score -= input.dependencies.reduce((total, dependency) => total + dependency.advisories * 5, 0);

  if (input.tests.failed > 0) {
    score -= 10;
  }

  return Math.max(0, Math.min(100, Math.round(score)));
}

export function riskRating(score: number): RiskRating {
  if (score >= 90) return 'Low';
  if (score >= 70) return 'Moderate';
  if (score >= 40) return 'Elevated';
  return 'Critical';
}

export function buildSecurityChecklist(input: AuditReportInput): ChecklistItem[] {
  const counts = countBySeverity(openFindings(input.findings));
  const advisories = input.dependencies.reduce((total, dependency) => total + dependency.advisories, 0);

  return [
    {
      id: 'no-critical',
      label: 'No open critical findings',
      passed: counts.critical === 0,
      detail: `${counts.critical} open critical finding(s)`,
    },
    {
      id: 'no-high',
      label: 'No open high-severity findings',
      passed: counts.high === 0,
      detail: `${counts.high} open high-severity finding(s)`,
    },
    {
      id: 'line-coverage',
      label: `Line coverage at or above ${MIN_LINE_COVERAGE}%`,
      passed: input.coverage.lines >= MIN_LINE_COVERAGE,
      detail: formatPercent(input.coverage.lines),
    },
    {
      id: 'branch-coverage',
      label: `Branch coverage at or above ${MIN_BRANCH_COVERAGE}%`,
      passed: input.coverage.branches >= MIN_BRANCH_COVERAGE,
      detail: formatPercent(input.coverage.branches),
    },
    {
      id: 'tests-passing',
      label: 'Full test suite passing',
      passed: input.tests.failed === 0 && input.tests.total > 0,
      detail: `${input.tests.passed}/${input.tests.total} passed`,
    },
    {
      id: 'dependency-advisories',
      label: 'No outstanding dependency advisories',
      passed: advisories === 0,
      detail: `${advisories} advisory/advisories across ${input.dependencies.length} package(s)`,
    },
  ];
}

export function buildExecutiveSummary(
  input: AuditReportInput,
  riskScore: number,
  rating: RiskRating,
): string {
  const open = openFindings(input.findings);
  const counts = countBySeverity(open);
  const breakdown = SEVERITY_ORDER.filter((severity) => counts[severity] > 0)
    .map((severity) => `${counts[severity]} ${severity}`)
    .join(', ');

  return [
    `${input.contractName} was analysed on ${input.network} by ${input.auditor}.`,
    `The audit recorded ${input.findings.length} static-analysis finding(s), of which ${open.length} remain open${breakdown ? ` (${breakdown})` : ''}.`,
    `Test coverage stands at ${formatPercent(input.coverage.lines)} of lines with ${input.tests.passed}/${input.tests.total} tests passing.`,
    `The resulting security posture score is ${riskScore}/100, rated ${rating}.`,
  ].join(' ');
}

export function buildAuditReport(input: AuditReportInput): AuditReport {
  const riskScore = computeRiskScore(input);
  const rating = riskRating(riskScore);

  return {
    ...input,
    severityCounts: countBySeverity(input.findings),
    openFindings: openFindings(input.findings).length,
    riskScore,
    riskRating: rating,
    checklist: buildSecurityChecklist(input),
    executiveSummary: buildExecutiveSummary(input, riskScore, rating),
  };
}

const markdownTable = (headers: string[], rows: string[][]): string =>
  [
    `| ${headers.join(' | ')} |`,
    `| ${headers.map(() => '---').join(' | ')} |`,
    ...rows.map((row) => `| ${row.join(' | ')} |`),
  ].join('\n');

export function formatMarkdownReport(report: AuditReport): string {
  const sections: string[] = [
    `# Contract Audit Report — ${report.contractName}`,
    markdownTable(
      ['Field', 'Value'],
      [
        ['Contract ID', report.contractId],
        ['Network', report.network],
        ['Auditor', report.auditor],
        ['Generated', formatDate(report.generatedAt)],
        ['Security score', `${report.riskScore}/100 (${report.riskRating})`],
      ],
    ),
    `## Executive Summary\n\n${report.executiveSummary}`,
    `## Security Checklist\n\n${report.checklist
      .map((item) => `- [${item.passed ? 'x' : ' '}] ${item.label} — ${item.detail}`)
      .join('\n')}`,
  ];

  sections.push(
    report.findings.length === 0
      ? '## Static Analysis Findings\n\nNo findings were reported.'
      : `## Static Analysis Findings\n\n${markdownTable(
          ['Severity', 'ID', 'Title', 'Location', 'Status'],
          report.findings.map((finding) => [
            finding.severity.toUpperCase(),
            finding.id,
            finding.title,
            finding.location,
            finding.status,
          ]),
        )}`,
  );

  sections.push(
    `## Test Results\n\n${markdownTable(
      ['Metric', 'Value'],
      [
        ['Total', String(report.tests.total)],
        ['Passed', String(report.tests.passed)],
        ['Failed', String(report.tests.failed)],
        ['Skipped', String(report.tests.skipped)],
        ['Duration', formatDuration(report.tests.durationMs)],
        ['Line coverage', formatPercent(report.coverage.lines)],
        ['Branch coverage', formatPercent(report.coverage.branches)],
        ['Function coverage', formatPercent(report.coverage.functions)],
      ],
    )}`,
  );

  sections.push(
    `## Gas Expenditure\n\n${markdownTable(
      ['Function', 'Average', 'Max', 'Invocations'],
      report.gas.map((entry) => [
        entry.functionName,
        formatGas(entry.averageGas),
        formatGas(entry.maxGas),
        entry.invocations.toLocaleString('en-US'),
      ]),
    )}`,
  );

  sections.push(
    `## Dependency Security\n\n${markdownTable(
      ['Package', 'Version', 'Score', 'Advisories'],
      report.dependencies.map((dependency) => [
        dependency.name,
        dependency.version,
        `${dependency.score}/100`,
        String(dependency.advisories),
      ]),
    )}`,
  );

  return `${sections.join('\n\n')}\n`;
}

export function reportFileName(report: AuditReport, extension: 'md' | 'pdf'): string {
  const slug = report.contractName
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '');
  return `${slug || 'contract'}-audit-report.${extension}`;
}

const pdfStyles = StyleSheet.create({
  page: { padding: 40, fontSize: 10, color: '#1e293b', fontFamily: 'Helvetica' },
  title: { fontSize: 18, marginBottom: 4, color: '#0f172a' },
  subtitle: { fontSize: 9, marginBottom: 18, color: '#64748b' },
  sectionTitle: { fontSize: 12, marginTop: 16, marginBottom: 6, color: '#0f172a' },
  paragraph: { lineHeight: 1.5, marginBottom: 4 },
  row: { flexDirection: 'row', borderBottomWidth: 1, borderBottomColor: '#e2e8f0', paddingVertical: 4 },
  headerRow: { flexDirection: 'row', borderBottomWidth: 1, borderBottomColor: '#94a3b8', paddingVertical: 4 },
  cell: { flex: 1, paddingRight: 6 },
  badge: { fontSize: 9, color: '#475569' },
});

const PdfTable: React.FC<{ headers: string[]; rows: string[][] }> = ({ headers, rows }) => (
  <View>
    <View style={pdfStyles.headerRow}>
      {headers.map((header) => (
        <Text style={pdfStyles.cell} key={header}>
          {header}
        </Text>
      ))}
    </View>
    {rows.map((row) => (
      <View style={pdfStyles.row} key={row.join('|')}>
        {row.map((cell, index) => (
          <Text style={pdfStyles.cell} key={`${cell}-${index}`}>
            {cell}
          </Text>
        ))}
      </View>
    ))}
  </View>
);

export const AuditReportDocument: React.FC<{ report: AuditReport }> = ({ report }) => (
  <Document title={`${report.contractName} audit report`} author={report.auditor}>
    <Page size="A4" style={pdfStyles.page}>
      <Text style={pdfStyles.title}>Contract Audit Report — {report.contractName}</Text>
      <Text style={pdfStyles.subtitle}>
        {report.contractId} · {report.network} · generated {formatDate(report.generatedAt)}
      </Text>

      <Text style={pdfStyles.sectionTitle}>Executive Summary</Text>
      <Text style={pdfStyles.paragraph}>{report.executiveSummary}</Text>
      <Text style={pdfStyles.badge}>
        Security score: {report.riskScore}/100 ({report.riskRating})
      </Text>

      <Text style={pdfStyles.sectionTitle}>Security Checklist</Text>
      {report.checklist.map((item) => (
        <Text style={pdfStyles.paragraph} key={item.id}>
          {item.passed ? '[PASS]' : '[FAIL]'} {item.label} — {item.detail}
        </Text>
      ))}

      <Text style={pdfStyles.sectionTitle}>Static Analysis Findings</Text>
      <PdfTable
        headers={['Severity', 'ID', 'Title', 'Status']}
        rows={report.findings.map((finding) => [
          finding.severity.toUpperCase(),
          finding.id,
          finding.title,
          finding.status,
        ])}
      />

      <Text style={pdfStyles.sectionTitle}>Test Results</Text>
      <PdfTable
        headers={['Total', 'Passed', 'Failed', 'Skipped', 'Line coverage']}
        rows={[
          [
            String(report.tests.total),
            String(report.tests.passed),
            String(report.tests.failed),
            String(report.tests.skipped),
            formatPercent(report.coverage.lines),
          ],
        ]}
      />

      <Text style={pdfStyles.sectionTitle}>Gas Expenditure</Text>
      <PdfTable
        headers={['Function', 'Average', 'Max', 'Invocations']}
        rows={report.gas.map((entry) => [
          entry.functionName,
          formatGas(entry.averageGas),
          formatGas(entry.maxGas),
          String(entry.invocations),
        ])}
      />

      <Text style={pdfStyles.sectionTitle}>Dependency Security</Text>
      <PdfTable
        headers={['Package', 'Version', 'Score', 'Advisories']}
        rows={report.dependencies.map((dependency) => [
          dependency.name,
          dependency.version,
          `${dependency.score}/100`,
          String(dependency.advisories),
        ])}
      />
    </Page>
  </Document>
);

export async function renderReportPdf(report: AuditReport): Promise<Blob> {
  return pdf(<AuditReportDocument report={report} />).toBlob();
}

export function downloadBlob(blob: Blob, fileName: string): void {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = fileName;
  document.body.appendChild(anchor);
  anchor.click();
  document.body.removeChild(anchor);
  URL.revokeObjectURL(url);
}

export const SAMPLE_REPORT_INPUT: AuditReportInput = {
  contractName: 'Escrow Vault',
  contractId: 'CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC',
  network: 'testnet',
  auditor: 'Crucible Static Analyzer',
  generatedAt: '2026-08-29T12:00:00.000Z',
  findings: [
    {
      id: 'CRU-001',
      title: 'Unchecked arithmetic in fee calculation',
      severity: 'high',
      location: 'src/lib.rs:142',
      description: 'Fee multiplication may overflow for large deposits.',
      status: 'open',
    },
    {
      id: 'CRU-002',
      title: 'Missing require_auth on admin setter',
      severity: 'critical',
      location: 'src/admin.rs:38',
      description: 'Administrative setter does not assert caller authorisation.',
      status: 'resolved',
    },
    {
      id: 'CRU-003',
      title: 'Storage entry never bumped',
      severity: 'medium',
      location: 'src/state.rs:76',
      description: 'Instance storage TTL is not extended on write.',
      status: 'open',
    },
  ],
  coverage: { lines: 87.4, branches: 78.1, functions: 92.3 },
  tests: { total: 64, passed: 63, failed: 1, skipped: 0, durationMs: 4820 },
  gas: [
    { functionName: 'deposit', averageGas: 14_500, maxGas: 21_300, invocations: 1280 },
    { functionName: 'withdraw', averageGas: 18_900, maxGas: 26_700, invocations: 940 },
    { functionName: 'settle', averageGas: 52_100, maxGas: 71_400, invocations: 130 },
  ],
  dependencies: [
    { name: 'soroban-sdk', version: '25.0.0', score: 96, advisories: 0 },
    { name: 'serde', version: '1.0.210', score: 94, advisories: 0 },
    { name: 'vulnerable-crate', version: '0.4.2', score: 41, advisories: 2 },
  ],
};

export interface AuditReportExportProps {
  input?: AuditReportInput;
}

export const AuditReportExport: React.FC<AuditReportExportProps> = ({ input = SAMPLE_REPORT_INPUT }) => {
  const [status, setStatus] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const report = useMemo(() => buildAuditReport(input), [input]);
  const markdown = useMemo(() => formatMarkdownReport(report), [report]);

  const handleMarkdown = () => {
    downloadBlob(new Blob([markdown], { type: 'text/markdown' }), reportFileName(report, 'md'));
    setStatus(`Markdown report exported as ${reportFileName(report, 'md')}`);
  };

  const handlePdf = async () => {
    setBusy(true);
    setStatus(null);
    try {
      const blob = await renderReportPdf(report);
      downloadBlob(blob, reportFileName(report, 'pdf'));
      setStatus(`PDF report exported as ${reportFileName(report, 'pdf')}`);
    } catch (error) {
      setStatus(error instanceof Error ? `PDF export failed: ${error.message}` : 'PDF export failed');
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="audit-report-container" data-testid="audit-report-export">
      <div className="audit-report-header">
        <div className="audit-report-icon-wrapper">
          <FileText className="audit-report-icon" />
        </div>
        <div>
          <h2>Contract Audit Report</h2>
          <p>Export a stakeholder-ready summary of findings, coverage, gas and dependencies</p>
        </div>
      </div>

      <div className="audit-report-summary glass-panel">
        <div className="audit-report-score">
          <ShieldCheck size={18} />
          <span className="audit-report-score-value" data-testid="risk-score">
            {report.riskScore}/100
          </span>
          <span
            className={`audit-report-rating audit-report-rating--${report.riskRating.toLowerCase()}`}
            data-testid="risk-rating"
          >
            {report.riskRating}
          </span>
        </div>
        <p className="audit-report-executive" data-testid="executive-summary">
          {report.executiveSummary}
        </p>
      </div>

      <div className="audit-report-panel glass-panel">
        <h3 className="audit-report-section-title">Security Checklist</h3>
        <ul className="audit-report-checklist" data-testid="security-checklist">
          {report.checklist.map((item) => (
            <li
              className={item.passed ? 'passed' : 'failed'}
              key={item.id}
              data-testid={`checklist-${item.id}`}
            >
              <span className="audit-report-check">{item.passed ? 'PASS' : 'FAIL'}</span>
              <span>{item.label}</span>
              <span className="audit-report-check-detail">{item.detail}</span>
            </li>
          ))}
        </ul>
      </div>

      <div className="audit-report-actions">
        <button type="button" className="audit-report-btn" onClick={handleMarkdown} data-testid="export-markdown">
          <Download size={14} />
          Export Markdown
        </button>
        <button
          type="button"
          className="audit-report-btn audit-report-btn--primary"
          onClick={() => void handlePdf()}
          disabled={busy}
          data-testid="export-pdf"
        >
          <Download size={14} />
          {busy ? 'Rendering PDF…' : 'Export PDF'}
        </button>
      </div>

      {status && (
        <p className="audit-report-status" data-testid="export-status">
          {status}
        </p>
      )}

      <details className="audit-report-preview">
        <summary>Markdown preview</summary>
        <pre data-testid="markdown-preview">{markdown}</pre>
      </details>
    </div>
  );
};
