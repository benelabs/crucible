import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { AbiFormGenerator, AbiSpec } from './AbiFormGenerator';

const VALID_ADDRESS = `G${'A'.repeat(55)}`;
const OTHER_ADDRESS = `C${'B'.repeat(55)}`;

const parsePayload = () => JSON.parse(screen.getByTestId('abi-payload').textContent ?? '{}');

describe('AbiFormGenerator', () => {
  it('renders the sample ABI and its functions', () => {
    render(<AbiFormGenerator />);
    expect(screen.getByTestId('abi-form-generator')).toBeInTheDocument();
    expect(screen.getByTestId('select-fn-initialize')).toBeInTheDocument();
    expect(screen.getByTestId('select-fn-record_votes')).toBeInTheDocument();
  });

  it('generates one labelled input per argument, typed from the ABI', () => {
    render(<AbiFormGenerator />);
    fireEvent.click(screen.getByTestId('select-fn-fund'));
    expect(screen.getByTestId('field-from')).toBeInTheDocument();
    expect(screen.getByTestId('field-amount')).toBeInTheDocument();
    expect(screen.getByText('u128')).toBeInTheDocument();
  });

  it('builds a payload from valid input', () => {
    const onSubmit = vi.fn();
    render(<AbiFormGenerator onSubmit={onSubmit} />);
    fireEvent.click(screen.getByTestId('select-fn-fund'));

    fireEvent.change(screen.getByTestId('field-from'), { target: { value: VALID_ADDRESS } });
    fireEvent.change(screen.getByTestId('field-amount'), { target: { value: '2500' } });
    fireEvent.click(screen.getByTestId('abi-submit'));

    expect(screen.getByTestId('abi-success')).toBeInTheDocument();
    expect(onSubmit).toHaveBeenCalledWith({
      contract: 'Escrow',
      function: 'fund',
      args: { from: VALID_ADDRESS, amount: '2500' },
    });
    expect(parsePayload().args.amount).toBe('2500');
  });

  it('blocks submission and marks the field when an address is malformed', () => {
    const onSubmit = vi.fn();
    render(<AbiFormGenerator onSubmit={onSubmit} />);
    fireEvent.click(screen.getByTestId('select-fn-fund'));

    fireEvent.change(screen.getByTestId('field-from'), { target: { value: 'GABC' } });
    fireEvent.change(screen.getByTestId('field-amount'), { target: { value: '1' } });
    fireEvent.click(screen.getByTestId('abi-submit'));

    expect(onSubmit).not.toHaveBeenCalled();
    expect(screen.queryByTestId('abi-payload')).not.toBeInTheDocument();
    expect(screen.getByTestId('error-from')).toBeInTheDocument();
    expect(screen.getByTestId('abi-error-summary')).toHaveTextContent('1 validation error');
  });

  it('rejects a u128 argument that is not a whole number', () => {
    render(<AbiFormGenerator />);
    fireEvent.click(screen.getByTestId('select-fn-fund'));

    fireEvent.change(screen.getByTestId('field-from'), { target: { value: VALID_ADDRESS } });
    fireEvent.change(screen.getByTestId('field-amount'), { target: { value: '12.5' } });
    fireEvent.click(screen.getByTestId('abi-submit'));

    expect(screen.getByTestId('error-amount')).toHaveTextContent('whole number');
  });

  it('preserves a u128 too large for a JS number', () => {
    render(<AbiFormGenerator />);
    fireEvent.click(screen.getByTestId('select-fn-fund'));

    const big = '340282366920938463463374607431768211455';
    fireEvent.change(screen.getByTestId('field-from'), { target: { value: VALID_ADDRESS } });
    fireEvent.change(screen.getByTestId('field-amount'), { target: { value: big } });
    fireEvent.click(screen.getByTestId('abi-submit'));

    expect(parsePayload().args.amount).toBe(big);
  });

  it('enforces the exact length of a BytesN argument', () => {
    render(<AbiFormGenerator />);
    fireEvent.click(screen.getByTestId('select-fn-attach_receipt'));

    fireEvent.change(screen.getByTestId('field-digest'), { target: { value: 'dead' } });
    fireEvent.click(screen.getByTestId('abi-submit'));
    expect(screen.getByTestId('error-digest')).toHaveTextContent('64 hex characters');

    fireEvent.change(screen.getByTestId('field-digest'), { target: { value: 'ab'.repeat(32) } });
    fireEvent.click(screen.getByTestId('abi-submit'));
    expect(screen.getByTestId('abi-success')).toBeInTheDocument();
  });

  it('renders a nested struct as a fieldset and submits it nested', () => {
    render(<AbiFormGenerator />);

    expect(screen.getByTestId('struct-terms')).toBeInTheDocument();
    fireEvent.change(screen.getByTestId('field-depositor'), { target: { value: VALID_ADDRESS } });
    fireEvent.change(screen.getByTestId('field-beneficiary'), { target: { value: OTHER_ADDRESS } });
    fireEvent.change(screen.getByTestId('field-terms.arbiter'), { target: { value: VALID_ADDRESS } });
    fireEvent.click(screen.getByTestId('abi-submit'));

    expect(parsePayload().args.terms).toEqual({
      arbiter: VALID_ADDRESS,
      milestones: [],
      memo: null,
    });
  });

  it('adds and removes items in a Vec of structs', () => {
    render(<AbiFormGenerator />);

    fireEvent.click(screen.getByTestId('field-terms.milestones-add'));
    fireEvent.change(screen.getByTestId('field-terms.milestones.[0].label'), {
      target: { value: 'kickoff' },
    });
    fireEvent.change(screen.getByTestId('field-terms.milestones.[0].amount'), {
      target: { value: '100' },
    });
    fireEvent.change(screen.getByTestId('field-terms.milestones.[0].deadline'), {
      target: { value: '1700000000' },
    });

    fireEvent.change(screen.getByTestId('field-depositor'), { target: { value: VALID_ADDRESS } });
    fireEvent.change(screen.getByTestId('field-beneficiary'), { target: { value: OTHER_ADDRESS } });
    fireEvent.change(screen.getByTestId('field-terms.arbiter'), { target: { value: VALID_ADDRESS } });
    fireEvent.click(screen.getByTestId('abi-submit'));

    expect(parsePayload().args.terms.milestones).toEqual([
      { label: 'kickoff', amount: '100', deadline: '1700000000' },
    ]);

    fireEvent.click(screen.getByTestId('field-terms.milestones-remove-0'));
    expect(screen.queryByTestId('field-terms.milestones.[0].label')).not.toBeInTheDocument();
  });

  it('reports an error against the offending element of a Vec', () => {
    render(<AbiFormGenerator />);

    fireEvent.click(screen.getByTestId('field-terms.milestones-add'));
    fireEvent.change(screen.getByTestId('field-terms.milestones.[0].label'), {
      target: { value: 'not a symbol!' },
    });
    fireEvent.click(screen.getByTestId('abi-submit'));

    expect(screen.getByTestId('error-terms.milestones.[0].label')).toBeInTheDocument();
  });

  it('toggles an Option between None and Some', () => {
    render(<AbiFormGenerator />);

    fireEvent.change(screen.getByTestId('field-depositor'), { target: { value: VALID_ADDRESS } });
    fireEvent.change(screen.getByTestId('field-beneficiary'), { target: { value: OTHER_ADDRESS } });
    fireEvent.change(screen.getByTestId('field-terms.arbiter'), { target: { value: VALID_ADDRESS } });

    fireEvent.click(screen.getByTestId('field-terms.memo-toggle'));
    fireEvent.change(screen.getByTestId('field-terms.memo'), { target: { value: 'q3 retainer' } });
    fireEvent.click(screen.getByTestId('abi-submit'));
    expect(parsePayload().args.terms.memo).toBe('q3 retainer');

    fireEvent.click(screen.getByTestId('field-terms.memo-toggle'));
    fireEvent.click(screen.getByTestId('abi-submit'));
    expect(parsePayload().args.terms.memo).toBeNull();
  });

  it('collects Map entries as key/value pairs and rejects duplicate keys', () => {
    render(<AbiFormGenerator />);
    fireEvent.click(screen.getByTestId('select-fn-record_votes'));

    fireEvent.click(screen.getByTestId('field-tally-add'));
    fireEvent.change(screen.getByTestId('field-tally.[0].key'), { target: { value: VALID_ADDRESS } });
    fireEvent.change(screen.getByTestId('field-tally.[0].value'), { target: { value: '3' } });
    fireEvent.click(screen.getByTestId('abi-submit'));

    expect(parsePayload().args.tally).toEqual([{ key: VALID_ADDRESS, value: '3' }]);

    fireEvent.click(screen.getByTestId('field-tally-add'));
    fireEvent.change(screen.getByTestId('field-tally.[1].key'), { target: { value: VALID_ADDRESS } });
    fireEvent.change(screen.getByTestId('field-tally.[1].value'), { target: { value: '4' } });
    fireEvent.click(screen.getByTestId('abi-submit'));

    expect(screen.getByTestId('error-tally.[1].key')).toHaveTextContent('Duplicate key');
  });

  it('offers enum variants as a select and submits the chosen one', () => {
    render(<AbiFormGenerator />);
    fireEvent.click(screen.getByTestId('select-fn-set_status'));

    fireEvent.change(screen.getByTestId('field-status'), { target: { value: 'Disputed' } });
    fireEvent.click(screen.getByTestId('abi-submit'));

    expect(parsePayload().args.status).toBe('Disputed');
  });

  it('clears state when a different function is selected', () => {
    render(<AbiFormGenerator />);
    fireEvent.click(screen.getByTestId('select-fn-fund'));
    fireEvent.change(screen.getByTestId('field-amount'), { target: { value: '999' } });

    fireEvent.click(screen.getByTestId('select-fn-attach_receipt'));
    fireEvent.click(screen.getByTestId('select-fn-fund'));

    expect(screen.getByTestId('field-amount')).toHaveValue('');
  });

  it('accepts a caller-supplied ABI and reports a function with no arguments', () => {
    const abi: AbiSpec = {
      name: 'Counter',
      functions: [{ name: 'increment', args: [], returnType: 'u32' }],
    };
    render(<AbiFormGenerator abi={abi} />);

    expect(screen.getByText('This function takes no arguments.')).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('abi-submit'));
    expect(parsePayload()).toEqual({ contract: 'Counter', function: 'increment', args: {} });
  });
});
