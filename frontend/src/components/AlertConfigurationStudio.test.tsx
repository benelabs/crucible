import { render, screen, fireEvent, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import {
  AlertConfigurationStudio,
  EMAIL_RELAY_ENDPOINT,
  RULE_SCHEMA,
  buildTestPayload,
  coerceConditionValue,
  createDefaultRule,
  describeRule,
  deserializeRule,
  deserializeRules,
  sendTestPing,
  serializeRule,
  serializeRules,
  validateChannel,
  validateRule,
  type AlertChannel,
  type AlertRule,
} from './AlertConfigurationStudio';

const rule = (overrides: Partial<AlertRule> = {}): AlertRule => ({
  id: 'rule-test',
  name: 'Panic watch',
  enabled: true,
  match: 'ANY',
  conditions: [
    { id: 'c1', field: 'event', operator: '==', value: 'reverted' },
    { id: 'c2', field: 'gas', operator: '>', value: '50000' },
  ],
  channels: [
    { id: 'ch1', type: 'email', target: 'oncall@example.com', enabled: true },
    { id: 'ch2', type: 'slack', target: 'https://hooks.slack.com/services/T/B/X', enabled: true },
    { id: 'ch3', type: 'discord', target: '', enabled: false },
  ],
  ...overrides,
});

const okResponse = (status = 204) =>
  vi.fn(async () => ({ ok: status < 400, status })) as unknown as typeof fetch;

describe('coerceConditionValue', () => {
  it('converts numeric fields to numbers', () => {
    expect(coerceConditionValue('gas', '50000')).toBe(50000);
    expect(coerceConditionValue('cpuInstructions', '1e6')).toBe(1_000_000);
  });

  it('leaves text fields as strings', () => {
    expect(coerceConditionValue('event', 'reverted')).toBe('reverted');
  });

  it('keeps unparseable numeric input as text so validation can report it', () => {
    expect(coerceConditionValue('gas', 'abc')).toBe('abc');
    expect(coerceConditionValue('gas', '')).toBe('');
  });
});

describe('serializeRule', () => {
  it('emits a versioned, JSON-safe rule with typed condition values', () => {
    expect(serializeRule(rule())).toEqual({
      schema: RULE_SCHEMA,
      id: 'rule-test',
      name: 'Panic watch',
      enabled: true,
      match: 'ANY',
      conditions: [
        { field: 'event', operator: '==', value: 'reverted' },
        { field: 'gas', operator: '>', value: 50000 },
      ],
      channels: [
        { type: 'email', target: 'oncall@example.com', enabled: true },
        { type: 'slack', target: 'https://hooks.slack.com/services/T/B/X', enabled: true },
        { type: 'discord', target: '', enabled: false },
      ],
    });
  });

  it('drops editor-only condition and channel ids', () => {
    const serialized = serializeRule(rule());

    expect(serialized.conditions[0]).not.toHaveProperty('id');
    expect(serialized.channels[0]).not.toHaveProperty('id');
  });

  it('survives a serialize/deserialize round trip', () => {
    const serialized = serializeRule(rule());

    expect(serializeRule(deserializeRule(serialized))).toEqual(serialized);
  });
});

describe('deserializeRule', () => {
  it('restores condition values as editable text', () => {
    const restored = deserializeRule(serializeRule(rule()));

    expect(restored.conditions.map((condition) => condition.value)).toEqual(['reverted', '50000']);
    expect(restored.match).toBe('ANY');
  });

  it('defaults an unknown match mode to ALL', () => {
    const restored = deserializeRule({
      ...serializeRule(rule()),
      match: 'SOMETHING' as never,
    });

    expect(restored.match).toBe('ALL');
  });

  it('rejects non-objects, foreign schemas, empty conditions and unknown fields', () => {
    expect(() => deserializeRule(null)).toThrow('Alert rule must be an object');
    expect(() => deserializeRule({ schema: 'other/v9' })).toThrow('Unsupported alert rule schema');
    expect(() => deserializeRule({ schema: RULE_SCHEMA, conditions: [] })).toThrow(
      'at least one condition',
    );
    expect(() =>
      deserializeRule({
        schema: RULE_SCHEMA,
        conditions: [{ field: 'nope', operator: '==', value: 1 }],
      }),
    ).toThrow('Unknown condition field: nope');
  });
});

describe('serializeRules', () => {
  it('round-trips a collection through JSON', () => {
    const json = serializeRules([rule(), rule({ id: 'rule-2', name: 'Whale transfers' })]);
    const restored = deserializeRules(json);

    expect(restored).toHaveLength(2);
    expect(restored.map((item) => item.name)).toEqual(['Panic watch', 'Whale transfers']);
    expect(serializeRules(restored)).toBe(json);
  });

  it('rejects JSON that is not a rule array', () => {
    expect(() => deserializeRules('{"schema":"x"}')).toThrow('Expected an array of alert rules');
  });
});

describe('describeRule', () => {
  it('renders the OR form quoting text values and leaving numbers bare', () => {
    expect(describeRule(rule())).toBe(
      'IF event == "reverted" OR gas > 50000 THEN notify email, slack',
    );
  });

  it('joins with AND when matching all conditions', () => {
    expect(describeRule(rule({ match: 'ALL' }))).toContain('"reverted" AND gas > 50000');
  });

  it('flags a rule with no enabled channel', () => {
    const channels = rule().channels.map((channel) => ({ ...channel, enabled: false }));

    expect(describeRule(rule({ channels }))).toContain('THEN notify (no channel)');
  });
});

describe('validateChannel', () => {
  it('requires a target', () => {
    expect(validateChannel({ id: 'x', type: 'slack', target: '  ', enabled: true })).toBe(
      'Slack target is required',
    );
  });

  it('requires an https webhook for slack and discord', () => {
    expect(
      validateChannel({ id: 'x', type: 'discord', target: 'http://insecure', enabled: true }),
    ).toBe('Discord webhook must be an https URL');
    expect(
      validateChannel({ id: 'x', type: 'slack', target: 'https://hooks.slack.com/x', enabled: true }),
    ).toBeNull();
  });

  it('requires a well-formed email address', () => {
    expect(validateChannel({ id: 'x', type: 'email', target: 'nope', enabled: true })).toBe(
      'nope is not a valid email address',
    );
    expect(
      validateChannel({ id: 'x', type: 'email', target: 'a@b.co', enabled: true }),
    ).toBeNull();
  });
});

describe('validateRule', () => {
  it('accepts a complete rule', () => {
    expect(validateRule(rule())).toEqual([]);
  });

  it('collects every problem it finds', () => {
    const errors = validateRule(
      rule({
        name: '  ',
        conditions: [
          { id: 'c1', field: 'gas', operator: '>', value: 'not-a-number' },
          { id: 'c2', field: 'event', operator: '==', value: '' },
        ],
        channels: [{ id: 'ch1', type: 'slack', target: '', enabled: false }],
      }),
    );

    expect(errors).toEqual([
      'Rule name is required',
      'Condition on gas must be numeric',
      'Condition on event needs a value',
      'Enable at least one notification channel',
    ]);
  });

  it('reports an empty condition list', () => {
    expect(validateRule(rule({ conditions: [] }))).toContain('Add at least one condition');
  });
});

describe('buildTestPayload', () => {
  it('uses each provider’s native message key', () => {
    const target = rule();

    expect(buildTestPayload(target, target.channels[1])).toEqual({
      text: '[Crucible test] Panic watch — IF event == "reverted" OR gas > 50000 THEN notify email, slack',
    });
    expect(buildTestPayload(target, target.channels[2])).toHaveProperty('content');
    expect(buildTestPayload(target, target.channels[0])).toMatchObject({
      to: 'oncall@example.com',
      subject: '[Crucible test] Panic watch',
    });
  });
});

describe('sendTestPing', () => {
  it('posts straight to a slack webhook and reports latency', async () => {
    const fetchImpl = okResponse();
    const clock = vi.fn().mockReturnValueOnce(0).mockReturnValueOnce(42);

    const result = await sendTestPing(rule(), rule().channels[1], fetchImpl, clock);

    expect(result).toEqual({ ok: true, status: 204, latencyMs: 42, error: undefined });
    expect(fetchImpl).toHaveBeenCalledWith(
      'https://hooks.slack.com/services/T/B/X',
      expect.objectContaining({ method: 'POST' }),
    );
  });

  it('routes email through the backend relay', async () => {
    const fetchImpl = okResponse(200);

    await sendTestPing(rule(), rule().channels[0], fetchImpl, () => 0);

    expect(fetchImpl).toHaveBeenCalledWith(EMAIL_RELAY_ENDPOINT, expect.anything());
  });

  it('refuses to send to an invalid target', async () => {
    const fetchImpl = okResponse();

    const result = await sendTestPing(rule(), rule().channels[2], fetchImpl, () => 0);

    expect(result).toMatchObject({ ok: false, error: 'Discord target is required' });
    expect(fetchImpl).not.toHaveBeenCalled();
  });

  it('reports webhook error responses', async () => {
    const result = await sendTestPing(rule(), rule().channels[1], okResponse(404), () => 0);

    expect(result).toMatchObject({ ok: false, status: 404, error: 'Webhook responded with HTTP 404' });
  });

  it('reports transport failures without throwing', async () => {
    const fetchImpl = vi.fn(async () => {
      throw new Error('Network down');
    }) as unknown as typeof fetch;

    await expect(sendTestPing(rule(), rule().channels[1], fetchImpl, () => 0)).resolves.toMatchObject({
      ok: false,
      status: null,
      error: 'Network down',
    });
  });
});

describe('AlertConfigurationStudio', () => {
  it('renders the default rule preview', () => {
    render(<AlertConfigurationStudio initialRules={[rule()]} />);

    expect(screen.getByText('Alert & Notification Studio')).toBeInTheDocument();
    expect(screen.getByTestId('rule-preview')).toHaveTextContent(
      'IF event == "reverted" OR gas > 50000 THEN notify email, slack',
    );
    expect(screen.queryByTestId('rule-errors')).not.toBeInTheDocument();
  });

  it('rebuilds the preview when a condition is edited', () => {
    render(<AlertConfigurationStudio initialRules={[rule()]} />);

    fireEvent.change(screen.getByTestId('value-c2'), { target: { value: '90000' } });
    fireEvent.click(screen.getByTestId('match-ALL'));

    expect(screen.getByTestId('rule-preview')).toHaveTextContent(
      'IF event == "reverted" AND gas > 90000 THEN notify email, slack',
    );
  });

  it('adds and removes conditions', () => {
    render(<AlertConfigurationStudio initialRules={[rule()]} />);

    fireEvent.click(screen.getByTestId('remove-c1'));
    expect(screen.getByTestId('rule-preview')).toHaveTextContent('IF gas > 50000 THEN');

    fireEvent.click(screen.getByTestId('add-condition'));
    expect(screen.getByTestId('rule-errors')).toHaveTextContent('Condition on gas needs a value');
  });

  it('keeps a compatible operator when switching a condition field', () => {
    render(<AlertConfigurationStudio initialRules={[rule()]} />);

    // gas > 50000 -> contractId only supports equality operators.
    fireEvent.change(screen.getByTestId('field-c2'), { target: { value: 'contractId' } });

    expect(screen.getByTestId('operator-c2')).toHaveValue('==');
  });

  it('sends a test ping and reports the delivery result', async () => {
    const fetchImpl = okResponse();
    render(<AlertConfigurationStudio initialRules={[rule()]} fetchImpl={fetchImpl} />);

    fireEvent.click(screen.getByTestId('test-ping-slack'));

    await waitFor(() => {
      expect(screen.getByTestId('ping-result-slack')).toHaveTextContent(/Delivered in/);
    });
    expect(fetchImpl).toHaveBeenCalledWith(
      'https://hooks.slack.com/services/T/B/X',
      expect.objectContaining({ method: 'POST' }),
    );
  });

  it('surfaces a failed test ping', async () => {
    render(<AlertConfigurationStudio initialRules={[rule()]} fetchImpl={okResponse(500)} />);

    fireEvent.click(screen.getByTestId('test-ping-email'));

    await waitFor(() => {
      expect(screen.getByTestId('ping-result-email')).toHaveTextContent(
        'Webhook responded with HTTP 500',
      );
    });
  });

  it('adds a new rule and switches selection to it', () => {
    render(<AlertConfigurationStudio initialRules={[rule()]} />);

    fireEvent.click(screen.getByTestId('add-rule'));

    expect(screen.getByTestId('rule-name')).toHaveValue('New rule');
    expect(screen.getAllByRole('listitem').length).toBeGreaterThan(1);
  });

  it('shows the serialized rule payload', () => {
    render(<AlertConfigurationStudio initialRules={[rule()]} />);

    expect(screen.getByTestId('rule-json')).toHaveTextContent(RULE_SCHEMA);
  });

  it('falls back to a generated rule when none are provided', () => {
    render(<AlertConfigurationStudio />);

    expect(screen.getByTestId('rule-name')).toHaveValue(createDefaultRule().name);
  });
});
