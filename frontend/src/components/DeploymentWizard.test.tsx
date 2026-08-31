import { render, screen, fireEvent } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import {
  ACCOUNT_RESERVE_XLM,
  BASE_FEE_XLM,
  DeploymentWizard,
  FEE_PER_KB_XLM,
  MAX_WASM_BYTES,
  SAMPLE_ABI,
  WASM_MAGIC,
  WASM_VERSION,
  canAdvance,
  checkDeployerBalance,
  estimateDeploymentFee,
  runPreflight,
  validateConstructorArgs,
  validateScalar,
  validateWasm,
  type ContractAbi,
} from './DeploymentWizard';

const wasmOf = (extraBytes = 8) =>
  new Uint8Array([...WASM_MAGIC, ...WASM_VERSION, ...new Array(extraBytes).fill(0)]);

const VALID_ADDRESS = `G${'A'.repeat(55)}`;

const validArgs = { admin: VALID_ADDRESS, decimals: '7', name: 'usdc' };

describe('validateWasm', () => {
  it('accepts a well-formed module', () => {
    expect(validateWasm(wasmOf())).toEqual({ valid: true, size: 16 });
  });

  it('rejects a missing or empty artifact', () => {
    expect(validateWasm(null)).toMatchObject({ valid: false, error: 'No Wasm artifact selected' });
    expect(validateWasm(new Uint8Array())).toMatchObject({ valid: false });
  });

  it('rejects an artifact too short to hold a header', () => {
    expect(validateWasm(new Uint8Array(WASM_MAGIC))).toMatchObject({
      valid: false,
      error: 'Artifact is too small to be a Wasm module',
    });
  });

  it('rejects bad magic bytes', () => {
    const bytes = wasmOf();
    bytes[1] = 0x62;

    expect(validateWasm(bytes)).toMatchObject({
      valid: false,
      error: 'Artifact is not a Wasm module (bad magic bytes)',
    });
  });

  it('rejects an unsupported Wasm version', () => {
    const bytes = wasmOf();
    bytes[4] = 0x02;

    expect(validateWasm(bytes)).toMatchObject({ valid: false, error: 'Unsupported Wasm version' });
  });

  it('rejects an artifact over the size limit', () => {
    const oversized = new Uint8Array(MAX_WASM_BYTES + 1);
    oversized.set([...WASM_MAGIC, ...WASM_VERSION]);

    expect(validateWasm(oversized)).toMatchObject({
      valid: false,
      error: `Artifact exceeds the ${MAX_WASM_BYTES / 1024} KB contract size limit`,
    });
  });
});

describe('estimateDeploymentFee', () => {
  it('charges the base fee plus whole kilobytes', () => {
    expect(estimateDeploymentFee(0)).toBeCloseTo(BASE_FEE_XLM, 10);
    expect(estimateDeploymentFee(1)).toBeCloseTo(BASE_FEE_XLM + FEE_PER_KB_XLM, 10);
    expect(estimateDeploymentFee(2048)).toBeCloseTo(BASE_FEE_XLM + 2 * FEE_PER_KB_XLM, 10);
    // A partial kilobyte still rounds up.
    expect(estimateDeploymentFee(2049)).toBeCloseTo(BASE_FEE_XLM + 3 * FEE_PER_KB_XLM, 10);
  });
});

describe('checkDeployerBalance', () => {
  it('requires the fee on top of the account reserve', () => {
    const check = checkDeployerBalance(10, 0.5);

    expect(check).toEqual({
      sufficient: true,
      balance: 10,
      required: 0.5 + ACCOUNT_RESERVE_XLM,
      shortfall: 0,
    });
  });

  it('reports the shortfall when the balance is too low', () => {
    const check = checkDeployerBalance(1, 0.5);

    expect(check.sufficient).toBe(false);
    expect(check.shortfall).toBeCloseTo(0.5, 10);
  });

  it('rejects a balance that exactly equals the requirement', () => {
    // Strictly greater, so the account is not drained to the reserve floor.
    expect(checkDeployerBalance(1.5, 0.5).sufficient).toBe(false);
  });

  it('honours a custom reserve', () => {
    expect(checkDeployerBalance(1, 0.5, 0).sufficient).toBe(true);
  });
});

describe('validateScalar', () => {
  it('requires a value', () => {
    expect(validateScalar('u32', '   ')).toBe('is required');
  });

  it('validates unsigned integers and their range', () => {
    expect(validateScalar('u32', '7')).toBeNull();
    expect(validateScalar('u32', '-1')).toBe('must be an unsigned integer');
    expect(validateScalar('u32', '1.5')).toBe('must be an unsigned integer');
    expect(validateScalar('u32', '4294967296')).toBe('is out of range for u32');
  });

  it('validates signed integers and their range', () => {
    expect(validateScalar('i32', '-5')).toBeNull();
    expect(validateScalar('i32', '2147483648')).toBe('is out of range for i32');
  });

  it('validates 128-bit integers beyond Number precision', () => {
    expect(validateScalar('u128', '340282366920938463463374607431768211455')).toBeNull();
    expect(validateScalar('u128', '340282366920938463463374607431768211456')).toBe(
      'is out of range for u128',
    );
    expect(validateScalar('i128', '-170141183460469231731687303715884105728')).toBeNull();
  });

  it('validates booleans', () => {
    expect(validateScalar('bool', 'true')).toBeNull();
    expect(validateScalar('bool', 'False')).toBe('must be true or false');
  });

  it('validates Stellar addresses', () => {
    expect(validateScalar('address', VALID_ADDRESS)).toBeNull();
    expect(validateScalar('address', `C${'B'.repeat(55)}`)).toBeNull();
    expect(validateScalar('address', 'GABC')).toBe('must be a 56-character G… or C… address');
  });

  it('validates symbols', () => {
    expect(validateScalar('symbol', 'my_token1')).toBeNull();
    expect(validateScalar('symbol', 'has spaces')).toBe(
      'must be up to 32 alphanumeric or underscore characters',
    );
  });

  it('validates hexadecimal bytes', () => {
    expect(validateScalar('bytes', '0xdeadbeef')).toBeNull();
    expect(validateScalar('bytes', 'abc')).toBe('must have an even number of hex digits');
    expect(validateScalar('bytes', '0xzz')).toBe('must be hexadecimal');
  });

  it('accepts any non-empty string', () => {
    expect(validateScalar('string', 'anything at all')).toBeNull();
  });
});

describe('validateConstructorArgs', () => {
  it('accepts arguments matching the ABI', () => {
    expect(validateConstructorArgs(SAMPLE_ABI, validArgs)).toEqual([]);
  });

  it('reports a missing argument', () => {
    const withoutDecimals = { admin: validArgs.admin, name: validArgs.name };

    expect(validateConstructorArgs(SAMPLE_ABI, withoutDecimals)).toEqual([
      { name: 'decimals', message: 'decimals is missing' },
    ]);
  });

  it('reports a badly typed argument', () => {
    expect(validateConstructorArgs(SAMPLE_ABI, { ...validArgs, decimals: 'seven' })).toEqual([
      { name: 'decimals', message: 'decimals must be an unsigned integer' },
    ]);
  });

  it('reports an argument the constructor does not declare', () => {
    expect(validateConstructorArgs(SAMPLE_ABI, { ...validArgs, surprise: '1' })).toEqual([
      { name: 'surprise', message: 'surprise is not declared by __constructor' },
    ]);
  });

  it('collects several problems at once', () => {
    const abi: ContractAbi = {
      constructorName: 'init',
      parameters: [
        { name: 'owner', type: 'address' },
        { name: 'cap', type: 'u64' },
      ],
    };

    expect(validateConstructorArgs(abi, { owner: 'nope', cap: '-1' })).toHaveLength(2);
  });

  it('accepts a constructor that takes no arguments', () => {
    expect(validateConstructorArgs({ constructorName: 'init', parameters: [] }, {})).toEqual([]);
  });
});

describe('runPreflight', () => {
  it('passes every check for a valid deployment', () => {
    const report = runPreflight({ wasm: wasmOf(), abi: SAMPLE_ABI, args: validArgs, balanceXlm: 25 });

    expect(report.canDeploy).toBe(true);
    expect(report.checks.map((check) => check.status)).toEqual(['pass', 'pass', 'pass']);
  });

  it('fails and explains an invalid artifact', () => {
    const report = runPreflight({ wasm: null, abi: SAMPLE_ABI, args: validArgs, balanceXlm: 25 });

    expect(report.canDeploy).toBe(false);
    expect(report.checks[0]).toMatchObject({ status: 'fail', detail: 'No Wasm artifact selected' });
  });

  it('fails when the deployer cannot cover the fee', () => {
    const report = runPreflight({ wasm: wasmOf(), abi: SAMPLE_ABI, args: validArgs, balanceXlm: 0.5 });

    expect(report.canDeploy).toBe(false);
    expect(report.checks[1].status).toBe('fail');
    expect(report.checks[1].detail).toMatch(/short by/);
  });

  it('fails and lists every argument problem', () => {
    const report = runPreflight({
      wasm: wasmOf(),
      abi: SAMPLE_ABI,
      args: { admin: 'bad', decimals: 'x', name: 'usdc' },
      balanceXlm: 25,
    });

    expect(report.canDeploy).toBe(false);
    expect(report.checks[2].detail).toContain('admin must be a 56-character');
    expect(report.checks[2].detail).toContain('decimals must be an unsigned integer');
  });
});

describe('canAdvance', () => {
  const failing = runPreflight({ wasm: null, abi: SAMPLE_ABI, args: {}, balanceXlm: 0 });
  const passing = runPreflight({ wasm: wasmOf(), abi: SAMPLE_ABI, args: validArgs, balanceXlm: 25 });

  it('blocks leaving the artifact step on an invalid Wasm', () => {
    expect(canAdvance('artifact', failing)).toBe(false);
    expect(canAdvance('artifact', passing)).toBe(true);
  });

  it('blocks reaching deploy until every check passes', () => {
    expect(canAdvance('preflight', failing)).toBe(false);
    expect(canAdvance('preflight', passing)).toBe(true);
  });

  it('always allows leaving the parameters step so checks can be reviewed', () => {
    expect(canAdvance('parameters', failing)).toBe(true);
  });
});

describe('DeploymentWizard', () => {
  it('starts on the artifact step', () => {
    render(<DeploymentWizard />);

    expect(screen.getByTestId('panel-artifact')).toBeInTheDocument();
    expect(screen.getByTestId('step-artifact')).toHaveAttribute('aria-current', 'step');
    expect(screen.getByTestId('wizard-back')).toBeDisabled();
  });

  it('blocks advancing past an invalid artifact', () => {
    render(<DeploymentWizard wasm={null} />);

    expect(screen.getByTestId('artifact-error')).toHaveTextContent('No Wasm artifact selected');
    expect(screen.getByTestId('wizard-next')).toBeDisabled();
    expect(screen.getByTestId('wizard-blocked')).toBeInTheDocument();
  });

  it('shows failing pre-flight checks for empty arguments', () => {
    render(<DeploymentWizard />);

    fireEvent.click(screen.getByTestId('wizard-next'));
    fireEvent.click(screen.getByTestId('wizard-next'));

    expect(screen.getByTestId('check-wasm')).toHaveAttribute('data-status', 'pass');
    expect(screen.getByTestId('check-arguments')).toHaveAttribute('data-status', 'fail');
    expect(screen.getByTestId('wizard-next')).toBeDisabled();
  });

  it('reports an insufficient balance in pre-flight', () => {
    render(<DeploymentWizard balanceXlm={0.2} />);

    fireEvent.click(screen.getByTestId('wizard-next'));
    fireEvent.click(screen.getByTestId('wizard-next'));

    expect(screen.getByTestId('check-balance')).toHaveAttribute('data-status', 'fail');
    expect(screen.getByTestId('check-balance')).toHaveTextContent('short by');
  });

  it('unblocks the deploy step once valid arguments are entered', () => {
    render(<DeploymentWizard />);

    fireEvent.click(screen.getByTestId('wizard-next'));
    fireEvent.change(screen.getByTestId('arg-admin'), { target: { value: VALID_ADDRESS } });
    fireEvent.change(screen.getByTestId('arg-decimals'), { target: { value: '7' } });
    fireEvent.change(screen.getByTestId('arg-name'), { target: { value: 'usdc' } });
    fireEvent.click(screen.getByTestId('wizard-next'));

    expect(screen.getByTestId('check-arguments')).toHaveAttribute('data-status', 'pass');
    expect(screen.getByTestId('wizard-next')).not.toBeDisabled();
  });

  it('deploys with the entered arguments', () => {
    const onDeploy = vi.fn();
    render(<DeploymentWizard onDeploy={onDeploy} />);

    fireEvent.click(screen.getByTestId('wizard-next'));
    fireEvent.change(screen.getByTestId('arg-admin'), { target: { value: VALID_ADDRESS } });
    fireEvent.change(screen.getByTestId('arg-decimals'), { target: { value: '7' } });
    fireEvent.change(screen.getByTestId('arg-name'), { target: { value: 'usdc' } });
    fireEvent.click(screen.getByTestId('wizard-next'));
    fireEvent.click(screen.getByTestId('wizard-next'));
    fireEvent.click(screen.getByTestId('confirm-deploy'));

    expect(onDeploy).toHaveBeenCalledWith({ admin: VALID_ADDRESS, decimals: '7', name: 'usdc' });
    expect(screen.getByTestId('deploy-success')).toBeInTheDocument();
  });

  it('can step back to a previous stage', () => {
    render(<DeploymentWizard />);

    fireEvent.click(screen.getByTestId('wizard-next'));
    expect(screen.getByTestId('panel-parameters')).toBeInTheDocument();

    fireEvent.click(screen.getByTestId('wizard-back'));
    expect(screen.getByTestId('panel-artifact')).toBeInTheDocument();
  });
});
