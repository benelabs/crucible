import React, { useCallback, useMemo, useState } from 'react';
import { Bell, CheckCircle2, Hash, Mail, MessageSquare, Plus, Send, Trash2, XCircle } from 'lucide-react';
import './AlertConfigurationStudio.css';

export type ConditionField = 'event' | 'gas' | 'transferAmount' | 'cpuInstructions' | 'contractId';

export type ConditionOperator = '==' | '!=' | '>' | '>=' | '<' | '<=';

/** ALL joins conditions with AND, ANY joins them with OR. */
export type MatchMode = 'ALL' | 'ANY';

export type ChannelType = 'email' | 'slack' | 'discord';

export interface AlertCondition {
  id: string;
  field: ConditionField;
  operator: ConditionOperator;
  /** Always held as text while editing; serialization coerces numeric fields. */
  value: string;
}

export interface AlertChannel {
  id: string;
  type: ChannelType;
  target: string;
  enabled: boolean;
}

export interface AlertRule {
  id: string;
  name: string;
  enabled: boolean;
  match: MatchMode;
  conditions: AlertCondition[];
  channels: AlertChannel[];
}

export interface SerializedAlertRule {
  schema: string;
  id: string;
  name: string;
  enabled: boolean;
  match: MatchMode;
  conditions: { field: ConditionField; operator: ConditionOperator; value: string | number }[];
  channels: { type: ChannelType; target: string; enabled: boolean }[];
}

export const RULE_SCHEMA = 'crucible.alert-rule/v1';

const TEXT_OPERATORS: ConditionOperator[] = ['==', '!='];
const NUMERIC_OPERATORS: ConditionOperator[] = ['==', '!=', '>', '>=', '<', '<='];

export const CONDITION_FIELDS: Record<
  ConditionField,
  { label: string; valueType: 'text' | 'number'; operators: ConditionOperator[]; placeholder: string }
> = {
  event: { label: 'event', valueType: 'text', operators: TEXT_OPERATORS, placeholder: 'reverted' },
  gas: { label: 'gas', valueType: 'number', operators: NUMERIC_OPERATORS, placeholder: '50000' },
  transferAmount: { label: 'transferAmount', valueType: 'number', operators: NUMERIC_OPERATORS, placeholder: '1000000' },
  cpuInstructions: { label: 'cpuInstructions', valueType: 'number', operators: NUMERIC_OPERATORS, placeholder: '20000000' },
  contractId: { label: 'contractId', valueType: 'text', operators: TEXT_OPERATORS, placeholder: 'CDLZFC3SY...' },
};

export const CHANNEL_LABELS: Record<ChannelType, { label: string; placeholder: string }> = {
  email: { label: 'Email', placeholder: 'oncall@example.com' },
  slack: { label: 'Slack', placeholder: 'https://hooks.slack.com/services/T000/B000/XXXX' },
  discord: { label: 'Discord', placeholder: 'https://discord.com/api/webhooks/000/XXXX' },
};

/** Relay used for email delivery, since email has no viewer-side webhook. */
export const EMAIL_RELAY_ENDPOINT = 'http://localhost:3000/api/v1/alerts/test-delivery';

const EMAIL_PATTERN = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

let idCounter = 0;
const createId = (prefix: string): string => {
  idCounter += 1;
  return `${prefix}-${idCounter}`;
};

/** Numeric fields serialize as numbers so the backend rule engine can compare them directly. */
export function coerceConditionValue(field: ConditionField, value: string): string | number {
  if (CONDITION_FIELDS[field].valueType !== 'number') return value;
  const parsed = Number(value);
  return Number.isFinite(parsed) && value.trim() !== '' ? parsed : value;
}

export function serializeRule(rule: AlertRule): SerializedAlertRule {
  return {
    schema: RULE_SCHEMA,
    id: rule.id,
    name: rule.name,
    enabled: rule.enabled,
    match: rule.match,
    conditions: rule.conditions.map((condition) => ({
      field: condition.field,
      operator: condition.operator,
      value: coerceConditionValue(condition.field, condition.value),
    })),
    channels: rule.channels.map((channel) => ({
      type: channel.type,
      target: channel.target,
      enabled: channel.enabled,
    })),
  };
}

export function deserializeRule(input: unknown): AlertRule {
  if (typeof input !== 'object' || input === null) {
    throw new Error('Alert rule must be an object');
  }
  const raw = input as Partial<SerializedAlertRule>;
  if (raw.schema !== RULE_SCHEMA) {
    throw new Error(`Unsupported alert rule schema: ${String(raw.schema)}`);
  }
  if (!Array.isArray(raw.conditions) || raw.conditions.length === 0) {
    throw new Error('Alert rule must declare at least one condition');
  }

  return {
    id: raw.id ?? createId('rule'),
    name: raw.name ?? 'Untitled rule',
    enabled: raw.enabled ?? true,
    match: raw.match === 'ANY' ? 'ANY' : 'ALL',
    conditions: raw.conditions.map((condition) => {
      if (!(condition.field in CONDITION_FIELDS)) {
        throw new Error(`Unknown condition field: ${String(condition.field)}`);
      }
      return {
        id: createId('condition'),
        field: condition.field,
        operator: condition.operator,
        value: String(condition.value),
      };
    }),
    channels: (raw.channels ?? []).map((channel) => ({
      id: createId('channel'),
      type: channel.type,
      target: channel.target,
      enabled: channel.enabled,
    })),
  };
}

export function serializeRules(rules: AlertRule[]): string {
  return JSON.stringify(rules.map(serializeRule), null, 2);
}

export function deserializeRules(json: string): AlertRule[] {
  const parsed = JSON.parse(json);
  if (!Array.isArray(parsed)) {
    throw new Error('Expected an array of alert rules');
  }
  return parsed.map(deserializeRule);
}

/** Renders a rule as the readable `IF … THEN notify …` form shown in the studio. */
export function describeRule(rule: AlertRule): string {
  const joiner = rule.match === 'ANY' ? ' OR ' : ' AND ';
  const clauses = rule.conditions.map((condition) => {
    const value = coerceConditionValue(condition.field, condition.value);
    const rendered = typeof value === 'number' ? String(value) : `"${value}"`;
    return `${condition.field} ${condition.operator} ${rendered}`;
  });
  const targets = rule.channels.filter((channel) => channel.enabled).map((channel) => channel.type);
  const notify = targets.length > 0 ? `notify ${targets.join(', ')}` : 'notify (no channel)';
  return `IF ${clauses.join(joiner)} THEN ${notify}`;
}

export function validateChannel(channel: AlertChannel): string | null {
  if (channel.target.trim() === '') {
    return `${CHANNEL_LABELS[channel.type].label} target is required`;
  }
  if (channel.type === 'email') {
    return EMAIL_PATTERN.test(channel.target) ? null : `${channel.target} is not a valid email address`;
  }
  return channel.target.startsWith('https://')
    ? null
    : `${CHANNEL_LABELS[channel.type].label} webhook must be an https URL`;
}

export function validateRule(rule: AlertRule): string[] {
  const errors: string[] = [];

  if (rule.name.trim() === '') {
    errors.push('Rule name is required');
  }
  if (rule.conditions.length === 0) {
    errors.push('Add at least one condition');
  }

  for (const condition of rule.conditions) {
    if (condition.value.trim() === '') {
      errors.push(`Condition on ${condition.field} needs a value`);
      continue;
    }
    if (CONDITION_FIELDS[condition.field].valueType === 'number' && !Number.isFinite(Number(condition.value))) {
      errors.push(`Condition on ${condition.field} must be numeric`);
    }
  }

  const enabledChannels = rule.channels.filter((channel) => channel.enabled);
  if (enabledChannels.length === 0) {
    errors.push('Enable at least one notification channel');
  }
  for (const channel of enabledChannels) {
    const error = validateChannel(channel);
    if (error) errors.push(error);
  }

  return errors;
}

/** Channel-native body for a test delivery; Slack and Discord use different keys. */
export function buildTestPayload(rule: AlertRule, channel: AlertChannel): Record<string, unknown> {
  const summary = `[Crucible test] ${rule.name} — ${describeRule(rule)}`;
  if (channel.type === 'slack') return { text: summary };
  if (channel.type === 'discord') return { content: summary };
  return {
    to: channel.target,
    subject: `[Crucible test] ${rule.name}`,
    body: summary,
  };
}

export interface TestPingResult {
  ok: boolean;
  status: number | null;
  latencyMs: number;
  error?: string;
}

/**
 * Delivers a test notification. Slack and Discord post straight to their webhook,
 * email goes through the backend relay. Transport failures resolve, never throw.
 */
export async function sendTestPing(
  rule: AlertRule,
  channel: AlertChannel,
  fetchImpl: typeof fetch = fetch,
  clock: () => number = () => Date.now(),
): Promise<TestPingResult> {
  const invalid = validateChannel(channel);
  if (invalid) {
    return { ok: false, status: null, latencyMs: 0, error: invalid };
  }

  const endpoint = channel.type === 'email' ? EMAIL_RELAY_ENDPOINT : channel.target;
  const startedAt = clock();
  try {
    const response = await fetchImpl(endpoint, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(buildTestPayload(rule, channel)),
    });
    const latencyMs = Math.max(0, Math.round(clock() - startedAt));
    return {
      ok: response.ok,
      status: response.status,
      latencyMs,
      error: response.ok ? undefined : `Webhook responded with HTTP ${response.status}`,
    };
  } catch (error) {
    return {
      ok: false,
      status: null,
      latencyMs: Math.max(0, Math.round(clock() - startedAt)),
      error: error instanceof Error ? error.message : 'Delivery failed',
    };
  }
}

export function createDefaultRule(): AlertRule {
  return {
    id: createId('rule'),
    name: 'Contract panic watch',
    enabled: true,
    match: 'ANY',
    conditions: [
      { id: createId('condition'), field: 'event', operator: '==', value: 'reverted' },
      { id: createId('condition'), field: 'gas', operator: '>', value: '50000' },
    ],
    channels: [
      { id: createId('channel'), type: 'email', target: 'oncall@example.com', enabled: true },
      { id: createId('channel'), type: 'slack', target: '', enabled: false },
      { id: createId('channel'), type: 'discord', target: '', enabled: false },
    ],
  };
}

const CHANNEL_ICONS: Record<ChannelType, React.ReactNode> = {
  email: <Mail size={14} />,
  slack: <Hash size={14} />,
  discord: <MessageSquare size={14} />,
};

export interface AlertConfigurationStudioProps {
  initialRules?: AlertRule[];
  fetchImpl?: typeof fetch;
}

export const AlertConfigurationStudio: React.FC<AlertConfigurationStudioProps> = ({
  initialRules,
  fetchImpl,
}) => {
  const [rules, setRules] = useState<AlertRule[]>(() => initialRules ?? [createDefaultRule()]);
  const [selectedRuleId, setSelectedRuleId] = useState<string>(() => (initialRules ?? [])[0]?.id ?? '');
  const [pings, setPings] = useState<Record<string, TestPingResult | 'pending'>>({});

  const selectedRule = useMemo(
    () => rules.find((rule) => rule.id === selectedRuleId) ?? rules[0],
    [rules, selectedRuleId],
  );

  const updateSelected = useCallback(
    (updater: (rule: AlertRule) => AlertRule) => {
      setRules((previous) =>
        previous.map((rule) => (rule.id === selectedRule?.id ? updater(rule) : rule)),
      );
    },
    [selectedRule?.id],
  );

  const handleTestPing = useCallback(
    async (channel: AlertChannel) => {
      if (!selectedRule) return;
      setPings((previous) => ({ ...previous, [channel.id]: 'pending' }));
      const result = await sendTestPing(selectedRule, channel, fetchImpl ?? fetch);
      setPings((previous) => ({ ...previous, [channel.id]: result }));
    },
    [selectedRule, fetchImpl],
  );

  const errors = selectedRule ? validateRule(selectedRule) : [];

  if (!selectedRule) {
    return (
      <div className="alert-studio-container" data-testid="alert-configuration-studio">
        <p className="alert-studio-empty">No alert rules configured.</p>
      </div>
    );
  }

  return (
    <div className="alert-studio-container" data-testid="alert-configuration-studio">
      <div className="alert-studio-header">
        <div className="alert-studio-icon-wrapper">
          <Bell className="alert-studio-icon" />
        </div>
        <div>
          <h2>Alert &amp; Notification Studio</h2>
          <p>Route contract panics, large transfers and gas spikes to email, Slack or Discord</p>
        </div>
      </div>

      <div className="alert-studio-content">
        <aside className="alert-studio-rules glass-panel">
          <div className="alert-studio-rules-head">
            <h3 className="alert-studio-section-title">Rules</h3>
            <button
              type="button"
              className="alert-studio-icon-btn"
              onClick={() => {
                const rule = createDefaultRule();
                rule.name = 'New rule';
                setRules((previous) => [...previous, rule]);
                setSelectedRuleId(rule.id);
              }}
              aria-label="Add rule"
              data-testid="add-rule"
            >
              <Plus size={16} />
            </button>
          </div>

          <ul className="alert-studio-rule-list">
            {rules.map((rule) => (
              <li key={rule.id}>
                <button
                  type="button"
                  className={`alert-studio-rule-btn ${rule.id === selectedRule.id ? 'active' : ''}`}
                  onClick={() => setSelectedRuleId(rule.id)}
                  data-testid={`rule-${rule.id}`}
                >
                  <span className={`alert-studio-dot ${rule.enabled ? 'on' : 'off'}`} />
                  {rule.name}
                </button>
              </li>
            ))}
          </ul>
        </aside>

        <section className="alert-studio-editor glass-panel">
          <div className="alert-studio-field-row">
            <label className="alert-studio-label" htmlFor="alert-rule-name">
              Rule name
            </label>
            <input
              id="alert-rule-name"
              className="alert-studio-input"
              value={selectedRule.name}
              onChange={(event) => updateSelected((rule) => ({ ...rule, name: event.target.value }))}
              data-testid="rule-name"
            />
          </div>

          <label className="alert-studio-toggle">
            <input
              type="checkbox"
              checked={selectedRule.enabled}
              onChange={(event) => updateSelected((rule) => ({ ...rule, enabled: event.target.checked }))}
              data-testid="rule-enabled"
            />
            Rule enabled
          </label>

          <div className="alert-studio-match">
            <span className="alert-studio-label">Match</span>
            {(['ALL', 'ANY'] as MatchMode[]).map((mode) => (
              <button
                key={mode}
                type="button"
                className={`alert-studio-chip ${selectedRule.match === mode ? 'active' : ''}`}
                onClick={() => updateSelected((rule) => ({ ...rule, match: mode }))}
                aria-pressed={selectedRule.match === mode}
                data-testid={`match-${mode}`}
              >
                {mode === 'ALL' ? 'All (AND)' : 'Any (OR)'}
              </button>
            ))}
          </div>

          <h3 className="alert-studio-section-title">Conditions</h3>
          <ul className="alert-studio-condition-list">
            {selectedRule.conditions.map((condition) => (
              <li className="alert-studio-condition" key={condition.id} data-testid={`condition-${condition.id}`}>
                <select
                  className="alert-studio-select"
                  value={condition.field}
                  aria-label="Condition field"
                  onChange={(event) => {
                    const field = event.target.value as ConditionField;
                    const operators = CONDITION_FIELDS[field].operators;
                    updateSelected((rule) => ({
                      ...rule,
                      conditions: rule.conditions.map((item) =>
                        item.id === condition.id
                          ? {
                              ...item,
                              field,
                              operator: operators.includes(item.operator) ? item.operator : operators[0],
                            }
                          : item,
                      ),
                    }));
                  }}
                  data-testid={`field-${condition.id}`}
                >
                  {(Object.keys(CONDITION_FIELDS) as ConditionField[]).map((field) => (
                    <option key={field} value={field}>
                      {CONDITION_FIELDS[field].label}
                    </option>
                  ))}
                </select>

                <select
                  className="alert-studio-select alert-studio-select--narrow"
                  value={condition.operator}
                  aria-label="Condition operator"
                  onChange={(event) =>
                    updateSelected((rule) => ({
                      ...rule,
                      conditions: rule.conditions.map((item) =>
                        item.id === condition.id
                          ? { ...item, operator: event.target.value as ConditionOperator }
                          : item,
                      ),
                    }))
                  }
                  data-testid={`operator-${condition.id}`}
                >
                  {CONDITION_FIELDS[condition.field].operators.map((operator) => (
                    <option key={operator} value={operator}>
                      {operator}
                    </option>
                  ))}
                </select>

                <input
                  className="alert-studio-input"
                  value={condition.value}
                  aria-label="Condition value"
                  placeholder={CONDITION_FIELDS[condition.field].placeholder}
                  onChange={(event) =>
                    updateSelected((rule) => ({
                      ...rule,
                      conditions: rule.conditions.map((item) =>
                        item.id === condition.id ? { ...item, value: event.target.value } : item,
                      ),
                    }))
                  }
                  data-testid={`value-${condition.id}`}
                />

                <button
                  type="button"
                  className="alert-studio-icon-btn"
                  aria-label="Remove condition"
                  onClick={() =>
                    updateSelected((rule) => ({
                      ...rule,
                      conditions: rule.conditions.filter((item) => item.id !== condition.id),
                    }))
                  }
                  data-testid={`remove-${condition.id}`}
                >
                  <Trash2 size={14} />
                </button>
              </li>
            ))}
          </ul>

          <button
            type="button"
            className="alert-studio-btn"
            onClick={() =>
              updateSelected((rule) => ({
                ...rule,
                conditions: [
                  ...rule.conditions,
                  { id: createId('condition'), field: 'gas', operator: '>', value: '' },
                ],
              }))
            }
            data-testid="add-condition"
          >
            <Plus size={14} />
            Add condition
          </button>

          <h3 className="alert-studio-section-title">Notification channels</h3>
          <ul className="alert-studio-channel-list">
            {selectedRule.channels.map((channel) => {
              const ping = pings[channel.id];
              return (
                <li className="alert-studio-channel" key={channel.id}>
                  <label className="alert-studio-toggle">
                    <input
                      type="checkbox"
                      checked={channel.enabled}
                      onChange={(event) =>
                        updateSelected((rule) => ({
                          ...rule,
                          channels: rule.channels.map((item) =>
                            item.id === channel.id ? { ...item, enabled: event.target.checked } : item,
                          ),
                        }))
                      }
                      data-testid={`channel-enabled-${channel.type}`}
                    />
                    {CHANNEL_ICONS[channel.type]}
                    {CHANNEL_LABELS[channel.type].label}
                  </label>

                  <input
                    className="alert-studio-input"
                    value={channel.target}
                    aria-label={`${CHANNEL_LABELS[channel.type].label} target`}
                    placeholder={CHANNEL_LABELS[channel.type].placeholder}
                    onChange={(event) =>
                      updateSelected((rule) => ({
                        ...rule,
                        channels: rule.channels.map((item) =>
                          item.id === channel.id ? { ...item, target: event.target.value } : item,
                        ),
                      }))
                    }
                    data-testid={`channel-target-${channel.type}`}
                  />

                  <button
                    type="button"
                    className="alert-studio-btn"
                    onClick={() => void handleTestPing(channel)}
                    disabled={ping === 'pending'}
                    data-testid={`test-ping-${channel.type}`}
                  >
                    <Send size={14} />
                    {ping === 'pending' ? 'Sending…' : 'Test ping'}
                  </button>

                  {ping && ping !== 'pending' && (
                    <span
                      className={`alert-studio-ping ${ping.ok ? 'ok' : 'failed'}`}
                      data-testid={`ping-result-${channel.type}`}
                    >
                      {ping.ok ? <CheckCircle2 size={14} /> : <XCircle size={14} />}
                      {ping.ok ? `Delivered in ${ping.latencyMs} ms` : ping.error}
                    </span>
                  )}
                </li>
              );
            })}
          </ul>

          <h3 className="alert-studio-section-title">Preview</h3>
          <p className="alert-studio-preview" data-testid="rule-preview">
            {describeRule(selectedRule)}
          </p>

          {errors.length > 0 && (
            <ul className="alert-studio-errors" data-testid="rule-errors">
              {errors.map((error) => (
                <li key={error}>{error}</li>
              ))}
            </ul>
          )}

          <details className="alert-studio-json">
            <summary>Serialized rule</summary>
            <pre data-testid="rule-json">{JSON.stringify(serializeRule(selectedRule), null, 2)}</pre>
          </details>
        </section>
      </div>
    </div>
  );
};
