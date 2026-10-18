/**
 * Recursive Soroban ABI type parsing and validation.
 *
 * Contract ABIs describe argument types as strings — `u128`, `Option<Address>`,
 * `Map<Symbol, Vec<BalanceEntry>>`. To generate a form for an arbitrary ABI we
 * need those strings as a tree rather than opaque text, plus per-node
 * validation so a bad value is caught before it reaches the network.
 */

/** A parsed ABI type. */
export type AbiType =
  | { kind: 'bool' }
  | { kind: 'void' }
  | { kind: 'int'; signed: boolean; bits: 32 | 64 | 128 | 256 }
  | { kind: 'symbol' }
  | { kind: 'string' }
  | { kind: 'bytes' }
  | { kind: 'bytesN'; length: number }
  | { kind: 'address' }
  | { kind: 'timepoint' }
  | { kind: 'duration' }
  | { kind: 'option'; inner: AbiType }
  | { kind: 'vec'; inner: AbiType }
  | { kind: 'map'; key: AbiType; value: AbiType }
  | { kind: 'tuple'; items: AbiType[] }
  | { kind: 'struct'; name: string; fields: AbiStructField[] }
  | { kind: 'enum'; name: string; variants: string[] }
  | { kind: 'unknown'; raw: string };

export interface AbiStructField {
  name: string;
  type: AbiType;
}

/** Custom types an ABI declares alongside its functions. */
export type AbiTypeRegistry = Record<
  string,
  { kind: 'struct'; fields: Array<{ name: string; type: string }> } | { kind: 'enum'; variants: string[] }
>;

/** Guards against a struct that (directly or indirectly) contains itself. */
const MAX_TYPE_DEPTH = 12;

/**
 * Split on commas that sit outside any `<...>`, so `Map<Symbol, Vec<u32>>`
 * yields `['Symbol', 'Vec<u32>']` rather than splitting the inner generic.
 */
export function splitTopLevel(input: string): string[] {
  const parts: string[] = [];
  let depth = 0;
  let current = '';
  for (const char of input) {
    if (char === '<') depth++;
    else if (char === '>') depth--;
    if (char === ',' && depth === 0) {
      parts.push(current.trim());
      current = '';
      continue;
    }
    current += char;
  }
  if (current.trim() !== '') parts.push(current.trim());
  return parts;
}

/** Parse a single ABI type string into a tree. */
export function parseAbiType(raw: string, registry: AbiTypeRegistry = {}, depth = 0): AbiType {
  const type = raw.trim();

  if (depth > MAX_TYPE_DEPTH) return { kind: 'unknown', raw: type };

  const generic = /^(\w+)<(.*)>$/.exec(type);
  if (generic) {
    const [, outer, innerRaw] = generic;
    const args = splitTopLevel(innerRaw);
    switch (outer) {
      case 'Option':
        return { kind: 'option', inner: parseAbiType(args[0] ?? 'void', registry, depth + 1) };
      case 'Vec':
        return { kind: 'vec', inner: parseAbiType(args[0] ?? 'void', registry, depth + 1) };
      case 'Map':
        return {
          kind: 'map',
          key: parseAbiType(args[0] ?? 'Symbol', registry, depth + 1),
          value: parseAbiType(args[1] ?? 'void', registry, depth + 1),
        };
      case 'Tuple':
        return { kind: 'tuple', items: args.map((a) => parseAbiType(a, registry, depth + 1)) };
      case 'BytesN': {
        const length = Number.parseInt(args[0] ?? '', 10);
        return Number.isFinite(length) && length > 0
          ? { kind: 'bytesN', length }
          : { kind: 'unknown', raw: type };
      }
      default:
        return { kind: 'unknown', raw: type };
    }
  }

  switch (type) {
    case 'bool':
      return { kind: 'bool' };
    case 'void':
    case '()':
      return { kind: 'void' };
    case 'u32':
      return { kind: 'int', signed: false, bits: 32 };
    case 'i32':
      return { kind: 'int', signed: true, bits: 32 };
    case 'u64':
      return { kind: 'int', signed: false, bits: 64 };
    case 'i64':
      return { kind: 'int', signed: true, bits: 64 };
    case 'u128':
      return { kind: 'int', signed: false, bits: 128 };
    case 'i128':
      return { kind: 'int', signed: true, bits: 128 };
    case 'u256':
      return { kind: 'int', signed: false, bits: 256 };
    case 'i256':
      return { kind: 'int', signed: true, bits: 256 };
    case 'Symbol':
      return { kind: 'symbol' };
    case 'String':
      return { kind: 'string' };
    case 'Bytes':
      return { kind: 'bytes' };
    case 'Address':
      return { kind: 'address' };
    case 'Timepoint':
      return { kind: 'timepoint' };
    case 'Duration':
      return { kind: 'duration' };
    default:
      break;
  }

  const custom = registry[type];
  if (custom?.kind === 'struct') {
    return {
      kind: 'struct',
      name: type,
      fields: custom.fields.map((f) => ({ name: f.name, type: parseAbiType(f.type, registry, depth + 1) })),
    };
  }
  if (custom?.kind === 'enum') {
    return { kind: 'enum', name: type, variants: custom.variants };
  }

  return { kind: 'unknown', raw: type };
}

/** Inclusive bounds for each supported integer width. */
export function integerBounds(signed: boolean, bits: number): { min: bigint; max: bigint } {
  if (signed) {
    const max = (1n << BigInt(bits - 1)) - 1n;
    return { min: -(1n << BigInt(bits - 1)), max };
  }
  return { min: 0n, max: (1n << BigInt(bits)) - 1n };
}

const STRKEY_BODY = /^[A-Z2-7]{55}$/;
const SYMBOL_RE = /^[A-Za-z0-9_]{1,32}$/;
const HEX_RE = /^[0-9a-fA-F]*$/;

/**
 * Stellar strkeys are base32 (RFC 4648, no padding): 56 characters, with the
 * first identifying the kind. Contract calls take account (`G`) or contract
 * (`C`) addresses. This checks shape and alphabet, not the trailing CRC —
 * enough to catch typos and paste errors before a round trip.
 */
export function isValidAddress(value: string): boolean {
  if (value.length !== 56) return false;
  const prefix = value[0];
  if (prefix !== 'G' && prefix !== 'C') return false;
  return STRKEY_BODY.test(value.slice(1));
}

/** The value shape the form holds for a given type. Leaves are strings. */
export type AbiValue = string | boolean | null | AbiValue[] | { [key: string]: AbiValue };

/** A validation failure, addressed by its path through the value tree. */
export interface AbiValidationError {
  path: string;
  message: string;
}

function joinPath(base: string, segment: string): string {
  return base === '' ? segment : `${base}.${segment}`;
}

/**
 * Validate a form value against its type, collecting every failure rather than
 * stopping at the first, so the form can mark all offending fields at once.
 */
export function validateAbiValue(type: AbiType, value: AbiValue, path = ''): AbiValidationError[] {
  const errors: AbiValidationError[] = [];
  const fail = (message: string) => errors.push({ path, message });

  switch (type.kind) {
    case 'void':
      break;

    case 'bool':
      if (typeof value !== 'boolean') fail('Expected a boolean.');
      break;

    case 'int': {
      const raw = String(value ?? '').trim();
      if (raw === '') {
        fail('Value is required.');
        break;
      }
      if (!/^-?\d+$/.test(raw)) {
        fail('Must be a whole number with no decimal point or exponent.');
        break;
      }
      const parsed = BigInt(raw);
      const { min, max } = integerBounds(type.signed, type.bits);
      if (parsed < min) fail(`Below the minimum for ${type.signed ? 'i' : 'u'}${type.bits} (${min}).`);
      else if (parsed > max) fail(`Exceeds the maximum for ${type.signed ? 'i' : 'u'}${type.bits} (${max}).`);
      break;
    }

    case 'timepoint':
    case 'duration': {
      const raw = String(value ?? '').trim();
      if (raw === '') fail('Value is required.');
      else if (!/^\d+$/.test(raw)) fail('Must be a non-negative whole number of seconds.');
      else if (BigInt(raw) > integerBounds(false, 64).max) fail('Exceeds the maximum for u64.');
      break;
    }

    case 'address': {
      const raw = String(value ?? '').trim();
      if (raw === '') fail('Address is required.');
      else if (!isValidAddress(raw)) {
        fail('Must be a 56-character Stellar address starting with G (account) or C (contract).');
      }
      break;
    }

    case 'symbol': {
      const raw = String(value ?? '');
      if (raw === '') fail('Symbol is required.');
      else if (!SYMBOL_RE.test(raw)) fail('Symbols are 1–32 characters of letters, digits, or underscore.');
      break;
    }

    case 'string':
      if (typeof value !== 'string') fail('Expected text.');
      break;

    case 'bytes': {
      const raw = String(value ?? '').trim();
      if (!HEX_RE.test(raw)) fail('Must be hexadecimal.');
      else if (raw.length % 2 !== 0) fail('Hex must have an even number of characters.');
      break;
    }

    case 'bytesN': {
      const raw = String(value ?? '').trim();
      const expected = type.length * 2;
      if (!HEX_RE.test(raw)) fail('Must be hexadecimal.');
      else if (raw.length !== expected) {
        fail(`Expected exactly ${expected} hex characters (${type.length} bytes), got ${raw.length}.`);
      }
      break;
    }

    case 'option':
      // `null` is the None case and is always valid; anything else is Some(T).
      if (value !== null) errors.push(...validateAbiValue(type.inner, value, path));
      break;

    case 'vec': {
      if (!Array.isArray(value)) {
        fail('Expected a list.');
        break;
      }
      value.forEach((item, index) => {
        errors.push(...validateAbiValue(type.inner, item, joinPath(path, `[${index}]`)));
      });
      break;
    }

    case 'map': {
      if (!Array.isArray(value)) {
        fail('Expected a list of key/value entries.');
        break;
      }
      const seenKeys = new Set<string>();
      value.forEach((entry, index) => {
        const pair = entry as { key: AbiValue; value: AbiValue };
        const entryPath = joinPath(path, `[${index}]`);
        errors.push(...validateAbiValue(type.key, pair?.key ?? '', joinPath(entryPath, 'key')));
        errors.push(...validateAbiValue(type.value, pair?.value ?? '', joinPath(entryPath, 'value')));

        // A Soroban map cannot hold the same key twice; submitting one would
        // silently drop an entry, so surface it as a form error instead.
        const keyId = JSON.stringify(pair?.key ?? null);
        if (seenKeys.has(keyId)) {
          errors.push({ path: joinPath(entryPath, 'key'), message: 'Duplicate key.' });
        }
        seenKeys.add(keyId);
      });
      break;
    }

    case 'tuple': {
      if (!Array.isArray(value)) {
        fail('Expected a tuple.');
        break;
      }
      type.items.forEach((item, index) => {
        errors.push(...validateAbiValue(item, value[index] ?? '', joinPath(path, `[${index}]`)));
      });
      break;
    }

    case 'struct': {
      if (typeof value !== 'object' || value === null || Array.isArray(value)) {
        fail('Expected an object.');
        break;
      }
      const record = value as Record<string, AbiValue>;
      type.fields.forEach((field) => {
        errors.push(...validateAbiValue(field.type, record[field.name] ?? '', joinPath(path, field.name)));
      });
      break;
    }

    case 'enum': {
      const raw = String(value ?? '');
      if (!type.variants.includes(raw)) fail(`Must be one of: ${type.variants.join(', ')}.`);
      break;
    }

    case 'unknown':
      fail(`Unrecognised type "${type.raw}" — supply this argument as raw JSON.`);
      break;
  }

  return errors;
}

/** The empty value the form starts a given type at. */
export function defaultValueFor(type: AbiType): AbiValue {
  switch (type.kind) {
    case 'bool':
      return false;
    case 'option':
      return null;
    case 'vec':
    case 'map':
      return [];
    case 'tuple':
      return type.items.map((item) => defaultValueFor(item));
    case 'struct': {
      const record: Record<string, AbiValue> = {};
      type.fields.forEach((field) => {
        record[field.name] = defaultValueFor(field.type);
      });
      return record;
    }
    case 'enum':
      return type.variants[0] ?? '';
    default:
      return '';
  }
}

/**
 * Convert validated form state into the payload shape a contract call expects:
 * integers as decimal strings (they exceed Number.MAX_SAFE_INTEGER), Options as
 * `null` or the inner value, and Maps as plain objects keyed by their entries.
 */
export function buildPayloadValue(type: AbiType, value: AbiValue): unknown {
  switch (type.kind) {
    case 'void':
      return null;
    case 'bool':
      return Boolean(value);
    case 'int':
    case 'timepoint':
    case 'duration':
      return String(value ?? '').trim();
    case 'option':
      return value === null ? null : buildPayloadValue(type.inner, value);
    case 'vec':
      return (value as AbiValue[]).map((item) => buildPayloadValue(type.inner, item));
    case 'map': {
      const entries = value as unknown as Array<{ key: AbiValue; value: AbiValue }>;
      return entries.map((entry) => ({
        key: buildPayloadValue(type.key, entry.key),
        value: buildPayloadValue(type.value, entry.value),
      }));
    }
    case 'tuple':
      return type.items.map((item, index) => buildPayloadValue(item, (value as AbiValue[])[index]));
    case 'struct': {
      const record = value as Record<string, AbiValue>;
      const out: Record<string, unknown> = {};
      type.fields.forEach((field) => {
        out[field.name] = buildPayloadValue(field.type, record[field.name]);
      });
      return out;
    }
    default:
      return value;
  }
}

/** Human-readable rendering of a parsed type, for labels and tooltips. */
export function formatAbiType(type: AbiType): string {
  switch (type.kind) {
    case 'int':
      return `${type.signed ? 'i' : 'u'}${type.bits}`;
    case 'bytesN':
      return `BytesN<${type.length}>`;
    case 'option':
      return `Option<${formatAbiType(type.inner)}>`;
    case 'vec':
      return `Vec<${formatAbiType(type.inner)}>`;
    case 'map':
      return `Map<${formatAbiType(type.key)}, ${formatAbiType(type.value)}>`;
    case 'tuple':
      return `Tuple<${type.items.map(formatAbiType).join(', ')}>`;
    case 'struct':
    case 'enum':
      return type.name;
    case 'unknown':
      return type.raw;
    default:
      return type.kind === 'bytes' ? 'Bytes' : type.kind.charAt(0).toUpperCase() + type.kind.slice(1);
  }
}
