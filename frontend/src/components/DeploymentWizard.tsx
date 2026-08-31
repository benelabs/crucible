import React, { useMemo, useState } from 'react';
import { CheckCircle2, ChevronLeft, ChevronRight, Rocket, XCircle } from 'lucide-react';
import './DeploymentWizard.css';

export type ScalarType =
  | 'u32'
  | 'i32'
  | 'u64'
  | 'i64'
  | 'u128'
  | 'i128'
  | 'bool'
  | 'symbol'
  | 'string'
  | 'address'
  | 'bytes';

export interface AbiParameter {
  name: string;
  type: ScalarType;
}

export interface ContractAbi {
  constructorName: string;
  parameters: AbiParameter[];
}

/** Every Wasm module starts with the `\0asm` magic followed by a u32 version. */
export const WASM_MAGIC = [0x00, 0x61, 0x73, 0x6d];
export const WASM_VERSION = [0x01, 0x00, 0x00, 0x00];
/** Soroban rejects uploads above this size. */
export const MAX_WASM_BYTES = 64 * 1024;

/** Base network fee plus a per-kilobyte upload component, in XLM. */
export const BASE_FEE_XLM = 0.00001;
export const FEE_PER_KB_XLM = 0.0025;
/** Account reserve that cannot be spent on fees. */
export const ACCOUNT_RESERVE_XLM = 1;

export interface WasmValidation {
  valid: boolean;
  size: number;
  error?: string;
}

export function validateWasm(bytes: Uint8Array | null): WasmValidation {
  if (!bytes || bytes.length === 0) {
    return { valid: false, size: 0, error: 'No Wasm artifact selected' };
  }
  if (bytes.length < 8) {
    return { valid: false, size: bytes.length, error: 'Artifact is too small to be a Wasm module' };
  }
  if (!WASM_MAGIC.every((byte, index) => bytes[index] === byte)) {
    return { valid: false, size: bytes.length, error: 'Artifact is not a Wasm module (bad magic bytes)' };
  }
  if (!WASM_VERSION.every((byte, index) => bytes[index + 4] === byte)) {
    return { valid: false, size: bytes.length, error: 'Unsupported Wasm version' };
  }
  if (bytes.length > MAX_WASM_BYTES) {
    return {
      valid: false,
      size: bytes.length,
      error: `Artifact exceeds the ${MAX_WASM_BYTES / 1024} KB contract size limit`,
    };
  }
  return { valid: true, size: bytes.length };
}

/** Upload fee scales with artifact size, rounded up to whole kilobytes. */
export function estimateDeploymentFee(wasmSize: number): number {
  return BASE_FEE_XLM + Math.ceil(wasmSize / 1024) * FEE_PER_KB_XLM;
}

export interface BalanceCheck {
  sufficient: boolean;
  balance: number;
  required: number;
  shortfall: number;
}

/**
 * The signer must cover the fee on top of the unspendable account reserve,
 * otherwise the transaction is rejected after it has already been broadcast.
 */
export function checkDeployerBalance(
  balanceXlm: number,
  estimatedFeeXlm: number,
  reserveXlm: number = ACCOUNT_RESERVE_XLM,
): BalanceCheck {
  const required = estimatedFeeXlm + reserveXlm;
  const shortfall = Math.max(0, required - balanceXlm);
  return {
    sufficient: balanceXlm > required,
    balance: balanceXlm,
    required,
    shortfall,
  };
}

export interface ArgumentIssue {
  name: string;
  message: string;
}

const STELLAR_ADDRESS = /^[GC][A-Z2-7]{55}$/;
const SYMBOL_PATTERN = /^[a-zA-Z0-9_]{1,32}$/;
const HEX_PATTERN = /^(0x)?[0-9a-fA-F]*$/;

const INTEGER_RANGES: Partial<Record<ScalarType, { min: bigint; max: bigint }>> = {
  u32: { min: 0n, max: 4_294_967_295n },
  i32: { min: -2_147_483_648n, max: 2_147_483_647n },
  u64: { min: 0n, max: 18_446_744_073_709_551_615n },
  i64: { min: -9_223_372_036_854_775_808n, max: 9_223_372_036_854_775_807n },
  u128: { min: 0n, max: (1n << 128n) - 1n },
  i128: { min: -(1n << 127n), max: (1n << 127n) - 1n },
};

/** Validates one raw form value against its declared ABI type. */
export function validateScalar(type: ScalarType, raw: string): string | null {
  const value = raw.trim();
  if (value === '') return 'is required';

  const range = INTEGER_RANGES[type];
  if (range) {
    const signed = type.startsWith('i');
    if (!(signed ? /^-?\d+$/ : /^\d+$/).test(value)) {
      return `must be ${signed ? 'an' : 'an unsigned'} integer`;
    }
    const parsed = BigInt(value);
    if (parsed < range.min || parsed > range.max) {
      return `is out of range for ${type}`;
    }
    return null;
  }

  if (type === 'bool') {
    return value === 'true' || value === 'false' ? null : 'must be true or false';
  }
  if (type === 'address') {
    return STELLAR_ADDRESS.test(value) ? null : 'must be a 56-character G… or C… address';
  }
  if (type === 'symbol') {
    return SYMBOL_PATTERN.test(value) ? null : 'must be up to 32 alphanumeric or underscore characters';
  }
  if (type === 'bytes') {
    const hex = value.startsWith('0x') ? value.slice(2) : value;
    if (!HEX_PATTERN.test(value)) return 'must be hexadecimal';
    return hex.length % 2 === 0 ? null : 'must have an even number of hex digits';
  }
  return null;
}

/**
 * Lints the constructor arguments against the ABI: every declared parameter
 * must be present and well typed, and nothing extra may be passed.
 */
export function validateConstructorArgs(
  abi: ContractAbi,
  args: Record<string, string>,
): ArgumentIssue[] {
  const issues: ArgumentIssue[] = [];

  for (const parameter of abi.parameters) {
    const raw = args[parameter.name];
    if (raw === undefined) {
      issues.push({ name: parameter.name, message: `${parameter.name} is missing` });
      continue;
    }
    const problem = validateScalar(parameter.type, raw);
    if (problem) {
      issues.push({ name: parameter.name, message: `${parameter.name} ${problem}` });
    }
  }

  const declared = new Set(abi.parameters.map((parameter) => parameter.name));
  for (const name of Object.keys(args)) {
    if (!declared.has(name)) {
      issues.push({ name, message: `${name} is not declared by ${abi.constructorName}` });
    }
  }

  return issues;
}

export type CheckStatus = 'pass' | 'fail';

export interface PreflightCheck {
  id: string;
  label: string;
  status: CheckStatus;
  detail: string;
}

export interface PreflightInput {
  wasm: Uint8Array | null;
  abi: ContractAbi;
  args: Record<string, string>;
  balanceXlm: number;
}

export interface PreflightReport {
  checks: PreflightCheck[];
  canDeploy: boolean;
  estimatedFeeXlm: number;
}

export function runPreflight(input: PreflightInput): PreflightReport {
  const wasm = validateWasm(input.wasm);
  const estimatedFeeXlm = estimateDeploymentFee(wasm.size);
  const balance = checkDeployerBalance(input.balanceXlm, estimatedFeeXlm);
  const argumentIssues = validateConstructorArgs(input.abi, input.args);

  const checks: PreflightCheck[] = [
    {
      id: 'wasm',
      label: 'Wasm artifact is valid',
      status: wasm.valid ? 'pass' : 'fail',
      detail: wasm.valid ? `${wasm.size.toLocaleString('en-US')} bytes` : wasm.error!,
    },
    {
      id: 'balance',
      label: 'Deployer balance covers the network fee',
      status: balance.sufficient ? 'pass' : 'fail',
      detail: balance.sufficient
        ? `${balance.balance} XLM available, ${balance.required.toFixed(5)} XLM required`
        : `short by ${balance.shortfall.toFixed(5)} XLM (needs ${balance.required.toFixed(5)} XLM)`,
    },
    {
      id: 'arguments',
      label: 'Constructor arguments match the ABI',
      status: argumentIssues.length === 0 ? 'pass' : 'fail',
      detail:
        argumentIssues.length === 0
          ? `${input.abi.parameters.length} argument(s) validated`
          : argumentIssues.map((issue) => issue.message).join('; '),
    },
  ];

  return {
    checks,
    canDeploy: checks.every((check) => check.status === 'pass'),
    estimatedFeeXlm,
  };
}

export type WizardStep = 'artifact' | 'parameters' | 'preflight' | 'deploy';

export const WIZARD_STEPS: { id: WizardStep; label: string }[] = [
  { id: 'artifact', label: 'Artifact' },
  { id: 'parameters', label: 'Parameters' },
  { id: 'preflight', label: 'Pre-flight' },
  { id: 'deploy', label: 'Deploy' },
];

/** The deploy step is only reachable once every pre-flight check passes. */
export function canAdvance(step: WizardStep, report: PreflightReport): boolean {
  if (step === 'artifact') return report.checks[0].status === 'pass';
  if (step === 'preflight') return report.canDeploy;
  return true;
}

export const SAMPLE_ABI: ContractAbi = {
  constructorName: '__constructor',
  parameters: [
    { name: 'admin', type: 'address' },
    { name: 'decimals', type: 'u32' },
    { name: 'name', type: 'symbol' },
  ],
};

export interface DeploymentWizardProps {
  abi?: ContractAbi;
  wasm?: Uint8Array | null;
  balanceXlm?: number;
  onDeploy?: (args: Record<string, string>) => void;
}

const validWasmStub = new Uint8Array([...WASM_MAGIC, ...WASM_VERSION, 0x01, 0x02, 0x03]);

export const DeploymentWizard: React.FC<DeploymentWizardProps> = ({
  abi = SAMPLE_ABI,
  wasm = validWasmStub,
  balanceXlm = 25,
  onDeploy,
}) => {
  const [stepIndex, setStepIndex] = useState(0);
  const [args, setArgs] = useState<Record<string, string>>(() =>
    Object.fromEntries(abi.parameters.map((parameter) => [parameter.name, ''])),
  );
  const [deployed, setDeployed] = useState(false);

  const report = useMemo(
    () => runPreflight({ wasm, abi, args, balanceXlm }),
    [wasm, abi, args, balanceXlm],
  );

  const step = WIZARD_STEPS[stepIndex].id;
  const advanceAllowed = canAdvance(step, report);

  return (
    <div className="deploy-wizard-container" data-testid="deployment-wizard">
      <div className="deploy-wizard-header">
        <div className="deploy-wizard-icon-wrapper">
          <Rocket className="deploy-wizard-icon" />
        </div>
        <div>
          <h2>Deployment Wizard</h2>
          <p>Pre-flight checks run before anything is broadcast to the network</p>
        </div>
      </div>

      <ol className="deploy-wizard-steps">
        {WIZARD_STEPS.map((item, index) => (
          <li
            key={item.id}
            className={`deploy-wizard-step ${index === stepIndex ? 'active' : ''} ${index < stepIndex ? 'done' : ''}`}
            data-testid={`step-${item.id}`}
            aria-current={index === stepIndex ? 'step' : undefined}
          >
            <span className="deploy-wizard-step-index">{index + 1}</span>
            {item.label}
          </li>
        ))}
      </ol>

      <section className="deploy-wizard-panel">
        {step === 'artifact' && (
          <div data-testid="panel-artifact">
            <h3 className="deploy-wizard-section-title">Compiled artifact</h3>
            <dl className="deploy-wizard-facts">
              <div>
                <dt>Size</dt>
                <dd data-testid="wasm-size">{(wasm?.length ?? 0).toLocaleString('en-US')} bytes</dd>
              </div>
              <div>
                <dt>Estimated fee</dt>
                <dd data-testid="estimated-fee">{report.estimatedFeeXlm.toFixed(5)} XLM</dd>
              </div>
              <div>
                <dt>Deployer balance</dt>
                <dd data-testid="deployer-balance">{balanceXlm} XLM</dd>
              </div>
            </dl>
            {report.checks[0].status === 'fail' && (
              <p className="deploy-wizard-error" data-testid="artifact-error">
                {report.checks[0].detail}
              </p>
            )}
          </div>
        )}

        {step === 'parameters' && (
          <div data-testid="panel-parameters">
            <h3 className="deploy-wizard-section-title">
              {abi.constructorName} arguments
            </h3>
            <div className="deploy-wizard-fields">
              {abi.parameters.map((parameter) => (
                <label className="deploy-wizard-field" key={parameter.name}>
                  <span className="deploy-wizard-field-label">
                    {parameter.name}
                    <code>{parameter.type}</code>
                  </span>
                  <input
                    className="deploy-wizard-input"
                    value={args[parameter.name] ?? ''}
                    onChange={(event) =>
                      setArgs((previous) => ({ ...previous, [parameter.name]: event.target.value }))
                    }
                    data-testid={`arg-${parameter.name}`}
                  />
                </label>
              ))}
            </div>
          </div>
        )}

        {step === 'preflight' && (
          <div data-testid="panel-preflight">
            <h3 className="deploy-wizard-section-title">Pre-flight checks</h3>
            <ul className="deploy-wizard-checks">
              {report.checks.map((check) => (
                <li
                  className={`deploy-wizard-check deploy-wizard-check--${check.status}`}
                  key={check.id}
                  data-testid={`check-${check.id}`}
                  data-status={check.status}
                >
                  {check.status === 'pass' ? <CheckCircle2 size={16} /> : <XCircle size={16} />}
                  <span className="deploy-wizard-check-label">{check.label}</span>
                  <span className="deploy-wizard-check-detail">{check.detail}</span>
                </li>
              ))}
            </ul>
          </div>
        )}

        {step === 'deploy' && (
          <div data-testid="panel-deploy">
            <h3 className="deploy-wizard-section-title">Ready to deploy</h3>
            <p className="deploy-wizard-ready">
              All pre-flight checks passed. Broadcasting costs about{' '}
              {report.estimatedFeeXlm.toFixed(5)} XLM.
            </p>
            <button
              type="button"
              className="deploy-wizard-btn deploy-wizard-btn--primary"
              onClick={() => {
                setDeployed(true);
                onDeploy?.(args);
              }}
              data-testid="confirm-deploy"
            >
              <Rocket size={14} />
              Deploy contract
            </button>
            {deployed && (
              <p className="deploy-wizard-success" data-testid="deploy-success">
                Deployment transaction submitted.
              </p>
            )}
          </div>
        )}
      </section>

      <div className="deploy-wizard-actions">
        <button
          type="button"
          className="deploy-wizard-btn"
          onClick={() => setStepIndex((index) => Math.max(0, index - 1))}
          disabled={stepIndex === 0}
          data-testid="wizard-back"
        >
          <ChevronLeft size={14} />
          Back
        </button>
        <button
          type="button"
          className="deploy-wizard-btn"
          onClick={() => setStepIndex((index) => Math.min(WIZARD_STEPS.length - 1, index + 1))}
          disabled={stepIndex === WIZARD_STEPS.length - 1 || !advanceAllowed}
          data-testid="wizard-next"
        >
          Next
          <ChevronRight size={14} />
        </button>
      </div>

      {!advanceAllowed && stepIndex < WIZARD_STEPS.length - 1 && (
        <p className="deploy-wizard-blocked" data-testid="wizard-blocked">
          Resolve the failing pre-flight checks before continuing.
        </p>
      )}
    </div>
  );
};
