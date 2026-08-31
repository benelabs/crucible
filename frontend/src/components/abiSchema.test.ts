import { describe, expect, it } from 'vitest';
import {
  AbiTypeRegistry,
  buildPayloadValue,
  defaultValueFor,
  formatAbiType,
  integerBounds,
  isValidAddress,
  parseAbiType,
  splitTopLevel,
  validateAbiValue,
} from './abiSchema';

const REGISTRY: AbiTypeRegistry = {
  Milestone: {
    kind: 'struct',
    fields: [
      { name: 'label', type: 'Symbol' },
      { name: 'amount', type: 'u128' },
    ],
  },
  Terms: {
    kind: 'struct',
    fields: [
      { name: 'arbiter', type: 'Address' },
      { name: 'milestones', type: 'Vec<Milestone>' },
    ],
  },
  Status: { kind: 'enum', variants: ['Draft', 'Funded'] },
};

const VALID_ADDRESS = `G${'A'.repeat(55)}`;

describe('splitTopLevel', () => {
  it('ignores commas nested inside generics', () => {
    expect(splitTopLevel('Symbol, Vec<u32>')).toEqual(['Symbol', 'Vec<u32>']);
    expect(splitTopLevel('Map<Address, u32>, bool')).toEqual(['Map<Address, u32>', 'bool']);
  });

  it('returns a single part when there is no comma', () => {
    expect(splitTopLevel('u128')).toEqual(['u128']);
  });
});

describe('parseAbiType', () => {
  it('parses primitives', () => {
    expect(parseAbiType('u128')).toEqual({ kind: 'int', signed: false, bits: 128 });
    expect(parseAbiType('i64')).toEqual({ kind: 'int', signed: true, bits: 64 });
    expect(parseAbiType('Address')).toEqual({ kind: 'address' });
    expect(parseAbiType('bool')).toEqual({ kind: 'bool' });
  });

  it('parses BytesN with its length', () => {
    expect(parseAbiType('BytesN<32>')).toEqual({ kind: 'bytesN', length: 32 });
  });

  it('parses containers recursively', () => {
    expect(parseAbiType('Vec<Option<Address>>')).toEqual({
      kind: 'vec',
      inner: { kind: 'option', inner: { kind: 'address' } },
    });
  });

  it('parses maps with distinct key and value types', () => {
    expect(parseAbiType('Map<Address, u32>')).toEqual({
      kind: 'map',
      key: { kind: 'address' },
      value: { kind: 'int', signed: false, bits: 32 },
    });
  });

  it('resolves custom structs from the registry, including nested ones', () => {
    const parsed = parseAbiType('Terms', REGISTRY);
    expect(parsed.kind).toBe('struct');
    if (parsed.kind !== 'struct') return;
    expect(parsed.fields[0]).toEqual({ name: 'arbiter', type: { kind: 'address' } });
    expect(parsed.fields[1].type).toEqual({
      kind: 'vec',
      inner: {
        kind: 'struct',
        name: 'Milestone',
        fields: [
          { name: 'label', type: { kind: 'symbol' } },
          { name: 'amount', type: { kind: 'int', signed: false, bits: 128 } },
        ],
      },
    });
  });

  it('resolves custom enums', () => {
    expect(parseAbiType('Status', REGISTRY)).toEqual({
      kind: 'enum',
      name: 'Status',
      variants: ['Draft', 'Funded'],
    });
  });

  it('falls back to unknown for types it cannot resolve', () => {
    expect(parseAbiType('SomeUnregisteredType')).toEqual({
      kind: 'unknown',
      raw: 'SomeUnregisteredType',
    });
  });

  it('stops recursing on a self-referential struct instead of hanging', () => {
    const recursive: AbiTypeRegistry = {
      Node: { kind: 'struct', fields: [{ name: 'next', type: 'Node' }] },
    };
    expect(() => parseAbiType('Node', recursive)).not.toThrow();
  });
});

describe('integerBounds', () => {
  it('computes unsigned bounds', () => {
    expect(integerBounds(false, 32)).toEqual({ min: 0n, max: 4294967295n });
    expect(integerBounds(false, 64).max).toBe(18446744073709551615n);
  });

  it('computes signed bounds', () => {
    expect(integerBounds(true, 32)).toEqual({ min: -2147483648n, max: 2147483647n });
  });
});

describe('isValidAddress', () => {
  it('accepts account and contract strkeys', () => {
    expect(isValidAddress(VALID_ADDRESS)).toBe(true);
    expect(isValidAddress(`C${'B'.repeat(55)}`)).toBe(true);
  });

  it('rejects wrong length, wrong prefix, and non-base32 characters', () => {
    expect(isValidAddress(`G${'A'.repeat(54)}`)).toBe(false);
    expect(isValidAddress(`X${'A'.repeat(55)}`)).toBe(false);
    expect(isValidAddress(`G${'1'.repeat(55)}`)).toBe(false);
  });
});

describe('validateAbiValue', () => {
  it('rejects an integer above its type maximum', () => {
    const errors = validateAbiValue(parseAbiType('u32'), '4294967296');
    expect(errors).toHaveLength(1);
    expect(errors[0].message).toContain('Exceeds the maximum');
  });

  it('accepts a u128 far beyond Number.MAX_SAFE_INTEGER', () => {
    expect(validateAbiValue(parseAbiType('u128'), '340282366920938463463374607431768211455')).toEqual([]);
  });

  it('rejects a negative value for an unsigned type', () => {
    const errors = validateAbiValue(parseAbiType('u64'), '-1');
    expect(errors[0].message).toContain('Below the minimum');
  });

  it('rejects non-integer text', () => {
    expect(validateAbiValue(parseAbiType('u32'), '1.5')[0].message).toContain('whole number');
  });

  it('validates addresses', () => {
    expect(validateAbiValue(parseAbiType('Address'), VALID_ADDRESS)).toEqual([]);
    expect(validateAbiValue(parseAbiType('Address'), 'not-an-address')).toHaveLength(1);
  });

  it('enforces the 32-character symbol limit', () => {
    expect(validateAbiValue(parseAbiType('Symbol'), 'a'.repeat(32))).toEqual([]);
    expect(validateAbiValue(parseAbiType('Symbol'), 'a'.repeat(33))).toHaveLength(1);
    expect(validateAbiValue(parseAbiType('Symbol'), 'has space')).toHaveLength(1);
  });

  it('enforces the exact byte length of BytesN', () => {
    expect(validateAbiValue(parseAbiType('BytesN<32>'), 'ab'.repeat(32))).toEqual([]);
    const short = validateAbiValue(parseAbiType('BytesN<32>'), 'abcd');
    expect(short[0].message).toContain('Expected exactly 64 hex characters');
  });

  it('treats None as valid and validates Some', () => {
    const option = parseAbiType('Option<Address>');
    expect(validateAbiValue(option, null)).toEqual([]);
    expect(validateAbiValue(option, 'bad')).toHaveLength(1);
  });

  it('reports errors per element with an indexed path', () => {
    const errors = validateAbiValue(parseAbiType('Vec<u32>'), ['1', '4294967296'], 'nums');
    expect(errors).toHaveLength(1);
    expect(errors[0].path).toBe('nums.[1]');
  });

  it('rejects duplicate map keys', () => {
    const errors = validateAbiValue(parseAbiType('Map<Symbol, u32>'), [
      { key: 'a', value: '1' },
      { key: 'a', value: '2' },
    ] as never, 'tally');
    expect(errors.some((e) => e.message === 'Duplicate key.')).toBe(true);
  });

  it('walks nested structs and reports the full path', () => {
    const terms = parseAbiType('Terms', REGISTRY);
    const errors = validateAbiValue(
      terms,
      { arbiter: VALID_ADDRESS, milestones: [{ label: 'ok', amount: 'oops' }] },
      'terms',
    );
    expect(errors).toHaveLength(1);
    expect(errors[0].path).toBe('terms.milestones.[0].amount');
  });

  it('collects every failure rather than stopping at the first', () => {
    const errors = validateAbiValue(parseAbiType('Vec<u32>'), ['x', 'y', 'z']);
    expect(errors).toHaveLength(3);
  });

  it('restricts enums to their declared variants', () => {
    const status = parseAbiType('Status', REGISTRY);
    expect(validateAbiValue(status, 'Draft')).toEqual([]);
    expect(validateAbiValue(status, 'Nope')).toHaveLength(1);
  });

  it('flags unknown types instead of silently accepting them', () => {
    expect(validateAbiValue(parseAbiType('Mystery'), 'x')[0].message).toContain('Unrecognised type');
  });
});

describe('defaultValueFor', () => {
  it('starts each kind at an empty but well-formed value', () => {
    expect(defaultValueFor(parseAbiType('bool'))).toBe(false);
    expect(defaultValueFor(parseAbiType('Option<u32>'))).toBeNull();
    expect(defaultValueFor(parseAbiType('Vec<u32>'))).toEqual([]);
    expect(defaultValueFor(parseAbiType('Status', REGISTRY))).toBe('Draft');
    expect(defaultValueFor(parseAbiType('Milestone', REGISTRY))).toEqual({ label: '', amount: '' });
  });
});

describe('buildPayloadValue', () => {
  it('keeps large integers as strings so precision survives', () => {
    const big = '340282366920938463463374607431768211455';
    expect(buildPayloadValue(parseAbiType('u128'), big)).toBe(big);
  });

  it('emits null for None and the inner value for Some', () => {
    const option = parseAbiType('Option<u32>');
    expect(buildPayloadValue(option, null)).toBeNull();
    expect(buildPayloadValue(option, '7')).toBe('7');
  });

  it('builds nested struct payloads', () => {
    const terms = parseAbiType('Terms', REGISTRY);
    expect(
      buildPayloadValue(terms, {
        arbiter: VALID_ADDRESS,
        milestones: [{ label: 'kickoff', amount: '100' }],
      }),
    ).toEqual({
      arbiter: VALID_ADDRESS,
      milestones: [{ label: 'kickoff', amount: '100' }],
    });
  });

  it('emits map entries as key/value pairs', () => {
    expect(
      buildPayloadValue(parseAbiType('Map<Symbol, u32>'), [{ key: 'a', value: '1' }] as never),
    ).toEqual([{ key: 'a', value: '1' }]);
  });
});

describe('formatAbiType', () => {
  it('round-trips a type back to readable text', () => {
    expect(formatAbiType(parseAbiType('Map<Address, Vec<u128>>'))).toBe('Map<Address, Vec<u128>>');
    expect(formatAbiType(parseAbiType('BytesN<32>'))).toBe('BytesN<32>');
    expect(formatAbiType(parseAbiType('Option<Symbol>'))).toBe('Option<Symbol>');
  });
});
