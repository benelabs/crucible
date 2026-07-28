import { describe, it, expect } from 'vitest';
import { ContractExecutionStateMachine } from './contractExecutionMachine';

describe('ContractExecutionStateMachine', () => {
  it('initializes in idle state', () => {
    const machine = new ContractExecutionStateMachine();
    expect(machine.getState()).toBe('idle');
  });

  it('handles successful state transitions', () => {
    const machine = new ContractExecutionStateMachine();

    machine.send({
      type: 'START_EXECUTION',
      payload: { functionName: 'deposit', inputs: { amount: '100' } },
    });
    expect(machine.getState()).toBe('validating');

    machine.send({ type: 'VALIDATED' });
    expect(machine.getState()).toBe('simulating');

    machine.send({ type: 'SIMULATION_SUCCESS', result: null });
    expect(machine.getState()).toBe('signing');

    machine.send({ type: 'SIGNED' });
    expect(machine.getState()).toBe('submitting');

    machine.send({ type: 'SUBMISSION_SUCCESS', result: { status: 'success' } });
    expect(machine.getState()).toBe('success');
    expect(machine.getContext().result).toEqual({ status: 'success' });
  });

  it('handles error state transitions', () => {
    const machine = new ContractExecutionStateMachine();

    machine.send({
      type: 'START_EXECUTION',
      payload: { functionName: 'transfer', inputs: {} },
    });
    expect(machine.getState()).toBe('validating');

    machine.send({ type: 'FAIL', error: 'Invalid parameters' });
    expect(machine.getState()).toBe('error');
    expect(machine.getContext().error).toBe('Invalid parameters');

    machine.send({ type: 'RESET' });
    expect(machine.getState()).toBe('idle');
  });
});
