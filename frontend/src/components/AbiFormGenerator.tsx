import React, { useCallback, useMemo, useState } from 'react';
import { AlertCircle, CheckCircle2, FileJson, Plus, Trash2, Upload } from 'lucide-react';
import './AbiFormGenerator.css';
import {
  buildPayloadValue,
  defaultValueFor,
  formatAbiType,
  parseAbiType,
  validateAbiValue,
} from './abiSchema';
import type { AbiType, AbiTypeRegistry, AbiValidationError, AbiValue } from './abiSchema';

export interface AbiFunctionSpec {
  name: string;
  args: Array<{ name: string; type: string }>;
  returnType?: string;
}

export interface AbiSpec {
  name: string;
  functions: AbiFunctionSpec[];
  types?: AbiTypeRegistry;
}

export interface AbiFormSubmission {
  contract: string;
  function: string;
  args: Record<string, unknown>;
}

interface AbiFormGeneratorProps {
  /** ABI to generate a form for. Falls back to a bundled sample ABI. */
  abi?: AbiSpec;
  onSubmit?: (submission: AbiFormSubmission) => void;
}

/**
 * A sample ABI exercising the cases hand-written JSON gets wrong most often:
 * a nested struct, an Option, a Vec of structs, and a Map.
 */
export const SAMPLE_ABI: AbiSpec = {
  name: 'Escrow',
  types: {
    Milestone: {
      kind: 'struct',
      fields: [
        { name: 'label', type: 'Symbol' },
        { name: 'amount', type: 'u128' },
        { name: 'deadline', type: 'Timepoint' },
      ],
    },
    Terms: {
      kind: 'struct',
      fields: [
        { name: 'arbiter', type: 'Address' },
        { name: 'milestones', type: 'Vec<Milestone>' },
        { name: 'memo', type: 'Option<String>' },
      ],
    },
    Status: { kind: 'enum', variants: ['Draft', 'Funded', 'Released', 'Disputed'] },
  },
  functions: [
    {
      name: 'initialize',
      args: [
        { name: 'depositor', type: 'Address' },
        { name: 'beneficiary', type: 'Address' },
        { name: 'terms', type: 'Terms' },
      ],
      returnType: 'void',
    },
    {
      name: 'fund',
      args: [
        { name: 'from', type: 'Address' },
        { name: 'amount', type: 'u128' },
      ],
      returnType: 'void',
    },
    {
      name: 'set_status',
      args: [{ name: 'status', type: 'Status' }],
      returnType: 'void',
    },
    {
      name: 'record_votes',
      args: [{ name: 'tally', type: 'Map<Address, u32>' }],
      returnType: 'u32',
    },
    {
      name: 'attach_receipt',
      args: [{ name: 'digest', type: 'BytesN<32>' }],
      returnType: 'void',
    },
  ],
};

/** Immutably write a value into the nested form state by path segments. */
function setAt(value: AbiValue, segments: Array<string | number>, next: AbiValue): AbiValue {
  if (segments.length === 0) return next;
  const [head, ...rest] = segments;
  if (typeof head === 'number') {
    const list = Array.isArray(value) ? [...value] : [];
    list[head] = setAt(list[head] ?? '', rest, next);
    return list;
  }
  const record = typeof value === 'object' && value !== null && !Array.isArray(value)
    ? { ...(value as Record<string, AbiValue>) }
    : {};
  record[head] = setAt(record[head] ?? '', rest, next);
  return record;
}

interface FieldProps {
  type: AbiType;
  value: AbiValue;
  path: Array<string | number>;
  pathKey: string;
  label: string;
  errors: Map<string, string>;
  onChange: (path: Array<string | number>, next: AbiValue) => void;
}

/**
 * Renders one node of the type tree. Composite types recurse, so an
 * arbitrarily nested ABI produces an arbitrarily nested form.
 */
const AbiField: React.FC<FieldProps> = ({ type, value, path, pathKey, label, errors, onChange }) => {
  const error = errors.get(pathKey);
  const testId = `field-${pathKey || 'root'}`;

  const scalarInput = (inputType: 'text' | 'number', placeholder: string) => (
    <input
      type={inputType}
      className={`afg-input ${error ? 'has-error' : ''}`}
      value={String(value ?? '')}
      placeholder={placeholder}
      aria-label={label}
      aria-invalid={error ? true : undefined}
      data-testid={testId}
      onChange={(e) => onChange(path, e.target.value)}
    />
  );

  let control: React.ReactNode;

  switch (type.kind) {
    case 'bool':
      control = (
        <label className="afg-checkbox">
          <input
            type="checkbox"
            checked={Boolean(value)}
            aria-label={label}
            data-testid={testId}
            onChange={(e) => onChange(path, e.target.checked)}
          />
          <span>{String(Boolean(value))}</span>
        </label>
      );
      break;

    case 'enum':
      control = (
        <select
          className="afg-input"
          value={String(value ?? '')}
          aria-label={label}
          data-testid={testId}
          onChange={(e) => onChange(path, e.target.value)}
        >
          {type.variants.map((variant) => (
            <option key={variant} value={variant}>
              {variant}
            </option>
          ))}
        </select>
      );
      break;

    case 'int':
      // Kept as text: u128/u256 exceed what a number input can represent
      // without losing precision.
      control = scalarInput('text', type.signed ? '-0' : '0');
      break;

    case 'address':
      control = scalarInput('text', 'G… or C…');
      break;

    case 'bytesN':
      control = scalarInput('text', `${type.length * 2} hex characters`);
      break;

    case 'option': {
      const enabled = value !== null;
      control = (
        <div className="afg-option">
          <label className="afg-checkbox">
            <input
              type="checkbox"
              checked={enabled}
              aria-label={`${label} present`}
              data-testid={`${testId}-toggle`}
              onChange={(e) => onChange(path, e.target.checked ? defaultValueFor(type.inner) : null)}
            />
            <span>{enabled ? 'Some' : 'None'}</span>
          </label>
          {enabled && (
            <AbiField
              type={type.inner}
              value={value}
              path={path}
              pathKey={pathKey}
              label={label}
              errors={errors}
              onChange={onChange}
            />
          )}
        </div>
      );
      break;
    }

    case 'vec': {
      const items = Array.isArray(value) ? value : [];
      control = (
        <div className="afg-collection">
          {items.map((item, index) => (
            <div className="afg-collection-row" key={index}>
              <AbiField
                type={type.inner}
                value={item}
                path={[...path, index]}
                pathKey={`${pathKey}.[${index}]`}
                label={`${label}[${index}]`}
                errors={errors}
                onChange={onChange}
              />
              <button
                type="button"
                className="afg-icon-btn"
                aria-label={`Remove ${label}[${index}]`}
                data-testid={`${testId}-remove-${index}`}
                onClick={() => onChange(path, items.filter((_, i) => i !== index))}
              >
                <Trash2 size={14} />
              </button>
            </div>
          ))}
          <button
            type="button"
            className="afg-add-btn"
            data-testid={`${testId}-add`}
            onClick={() => onChange(path, [...items, defaultValueFor(type.inner)])}
          >
            <Plus size={14} /> Add item
          </button>
        </div>
      );
      break;
    }

    case 'map': {
      const entries = (Array.isArray(value) ? value : []) as unknown as Array<{ key: AbiValue; value: AbiValue }>;
      control = (
        <div className="afg-collection">
          {entries.map((entry, index) => (
            <div className="afg-map-row" key={index}>
              <AbiField
                type={type.key}
                value={entry?.key ?? ''}
                path={[...path, index, 'key']}
                pathKey={`${pathKey}.[${index}].key`}
                label={`${label} key ${index}`}
                errors={errors}
                onChange={onChange}
              />
              <AbiField
                type={type.value}
                value={entry?.value ?? ''}
                path={[...path, index, 'value']}
                pathKey={`${pathKey}.[${index}].value`}
                label={`${label} value ${index}`}
                errors={errors}
                onChange={onChange}
              />
              <button
                type="button"
                className="afg-icon-btn"
                aria-label={`Remove ${label} entry ${index}`}
                data-testid={`${testId}-remove-${index}`}
                onClick={() => onChange(path, entries.filter((_, i) => i !== index) as unknown as AbiValue)}
              >
                <Trash2 size={14} />
              </button>
            </div>
          ))}
          <button
            type="button"
            className="afg-add-btn"
            data-testid={`${testId}-add`}
            onClick={() =>
              onChange(path, [
                ...entries,
                { key: defaultValueFor(type.key), value: defaultValueFor(type.value) },
              ] as unknown as AbiValue)
            }
          >
            <Plus size={14} /> Add entry
          </button>
        </div>
      );
      break;
    }

    case 'tuple':
      control = (
        <div className="afg-nested">
          {type.items.map((item, index) => (
            <AbiField
              key={index}
              type={item}
              value={(value as AbiValue[])?.[index] ?? ''}
              path={[...path, index]}
              pathKey={`${pathKey}.[${index}]`}
              label={`${label}.${index}`}
              errors={errors}
              onChange={onChange}
            />
          ))}
        </div>
      );
      break;

    case 'struct':
      control = (
        <fieldset className="afg-nested" data-testid={`struct-${pathKey}`}>
          <legend>{type.name}</legend>
          {type.fields.map((field) => (
            <AbiField
              key={field.name}
              type={field.type}
              value={(value as Record<string, AbiValue>)?.[field.name] ?? ''}
              path={[...path, field.name]}
              pathKey={pathKey ? `${pathKey}.${field.name}` : field.name}
              label={field.name}
              errors={errors}
              onChange={onChange}
            />
          ))}
        </fieldset>
      );
      break;

    case 'void':
      control = <span className="afg-muted">no value</span>;
      break;

    default:
      control = scalarInput('text', formatAbiType(type));
  }

  // Composite nodes label themselves through their legend or child rows.
  const showLabel = type.kind !== 'struct';

  return (
    <div className="afg-field">
      {showLabel && (
        <label className="afg-label">
          <span className="afg-label-name">{label}</span>
          <span className="afg-label-type">{formatAbiType(type)}</span>
        </label>
      )}
      {control}
      {error && (
        <span className="afg-error" role="alert" data-testid={`error-${pathKey || 'root'}`}>
          <AlertCircle size={12} /> {error}
        </span>
      )}
    </div>
  );
};

/**
 * Generates a validated input form for any function in a contract ABI, so
 * calling into nested structs no longer means hand-writing a JSON payload.
 */
export const AbiFormGenerator: React.FC<AbiFormGeneratorProps> = ({ abi = SAMPLE_ABI, onSubmit }) => {
  const [selected, setSelected] = useState(abi.functions[0]?.name ?? '');
  const [values, setValues] = useState<Record<string, AbiValue>>({});
  const [errors, setErrors] = useState<AbiValidationError[]>([]);
  const [payload, setPayload] = useState<AbiFormSubmission | null>(null);
  const [touched, setTouched] = useState(false);

  const fn = useMemo(
    () => abi.functions.find((f) => f.name === selected) ?? abi.functions[0],
    [abi.functions, selected],
  );

  const parsedArgs = useMemo(
    () => (fn?.args ?? []).map((arg) => ({ name: arg.name, type: parseAbiType(arg.type, abi.types ?? {}) })),
    [fn, abi.types],
  );

  // Arguments the user has not touched still need a well-formed starting value.
  const currentValues = useMemo(() => {
    const next: Record<string, AbiValue> = {};
    parsedArgs.forEach((arg) => {
      next[arg.name] = values[arg.name] ?? defaultValueFor(arg.type);
    });
    return next;
  }, [parsedArgs, values]);

  // Seeded per argument so that editing one field of a struct does not discard
  // its untouched siblings, which start life only as defaults.
  const defaults = useMemo(() => {
    const seeded: Record<string, AbiValue> = {};
    parsedArgs.forEach((arg) => {
      seeded[arg.name] = defaultValueFor(arg.type);
    });
    return seeded;
  }, [parsedArgs]);

  const handleChange = useCallback(
    (path: Array<string | number>, next: AbiValue) => {
      const [head, ...rest] = path;
      const key = head as string;
      setValues((prev) => ({
        ...prev,
        [key]: setAt(prev[key] ?? defaults[key] ?? '', rest, next),
      }));
    },
    [defaults],
  );

  const selectFunction = (name: string) => {
    setSelected(name);
    setValues({});
    setErrors([]);
    setPayload(null);
    setTouched(false);
  };

  const handleSubmit = (event: React.FormEvent) => {
    event.preventDefault();
    setTouched(true);

    const found: AbiValidationError[] = [];
    parsedArgs.forEach((arg) => {
      found.push(...validateAbiValue(arg.type, currentValues[arg.name], arg.name));
    });
    setErrors(found);

    if (found.length > 0) {
      setPayload(null);
      return;
    }

    const args: Record<string, unknown> = {};
    parsedArgs.forEach((arg) => {
      args[arg.name] = buildPayloadValue(arg.type, currentValues[arg.name]);
    });

    const submission: AbiFormSubmission = { contract: abi.name, function: fn?.name ?? '', args };
    setPayload(submission);
    onSubmit?.(submission);
  };

  const errorMap = useMemo(() => new Map(errors.map((e) => [e.path, e.message])), [errors]);

  return (
    <div className="afg-form-generator" data-testid="abi-form-generator">
      <header className="afg-header">
        <div className="afg-title">
          <FileJson size={20} />
          <div>
            <h2>ABI Form Generator</h2>
            <p>Typed inputs generated from {abi.name}&apos;s interface</p>
          </div>
        </div>
        <span className="afg-badge">
          <Upload size={12} /> {abi.functions.length} functions
        </span>
      </header>

      <div className="afg-function-picker" role="tablist" aria-label="Contract functions">
        {abi.functions.map((f) => (
          <button
            key={f.name}
            type="button"
            role="tab"
            aria-selected={f.name === fn?.name}
            className={`afg-function-btn ${f.name === fn?.name ? 'active' : ''}`}
            data-testid={`select-fn-${f.name}`}
            onClick={() => selectFunction(f.name)}
          >
            {f.name}
          </button>
        ))}
      </div>

      <form className="afg-form" onSubmit={handleSubmit} noValidate data-testid="abi-form">
        {parsedArgs.length === 0 && <p className="afg-muted">This function takes no arguments.</p>}

        {parsedArgs.map((arg) => (
          <AbiField
            key={arg.name}
            type={arg.type}
            value={currentValues[arg.name]}
            path={[arg.name]}
            pathKey={arg.name}
            label={arg.name}
            errors={errorMap}
            onChange={handleChange}
          />
        ))}

        <div className="afg-actions">
          <button type="submit" className="afg-submit" data-testid="abi-submit">
            Build payload
          </button>
          {touched && errors.length > 0 && (
            <span className="afg-status error" data-testid="abi-error-summary">
              <AlertCircle size={14} /> {errors.length} validation {errors.length === 1 ? 'error' : 'errors'}
            </span>
          )}
          {payload && (
            <span className="afg-status ok" data-testid="abi-success">
              <CheckCircle2 size={14} /> Payload ready
            </span>
          )}
        </div>
      </form>

      {payload && (
        <pre className="afg-payload" data-testid="abi-payload">
          {JSON.stringify(payload, null, 2)}
        </pre>
      )}
    </div>
  );
};

export default AbiFormGenerator;
