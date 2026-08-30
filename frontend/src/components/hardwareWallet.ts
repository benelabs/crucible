/**
 * Device-facing half of {@link HardwareWalletConnector} (issue #891).
 *
 * Kept out of the component so the connection state machine can be tested
 * without a device, and so a second vendor can be added by writing another
 * {@link HardwareWalletAdapter} rather than by editing the UI.
 */

export type HardwareVendor = 'ledger' | 'trezor';

export type ConnectionState =
  | 'idle'
  | 'requesting-device'
  | 'connecting'
  | 'fetching-key'
  | 'connected'
  | 'awaiting-confirmation'
  | 'error';

export type HardwareErrorCode =
  | 'unsupported'
  | 'no-device'
  | 'app-not-open'
  | 'user-rejected'
  | 'device-locked'
  | 'transport'
  | 'unknown';

export interface HardwareError {
  code: HardwareErrorCode;
  message: string;
  /** What the user should do about it. */
  remedy: string;
}

export interface DerivedAccount {
  derivationPath: string;
  publicKey: string;
}

export interface HardwareWalletAdapter {
  vendor: HardwareVendor;
  /** Open a transport and return a handle. Throws on failure. */
  connect(): Promise<void>;
  getPublicKey(derivationPath: string): Promise<string>;
  signTransaction(
    derivationPath: string,
    payload: Uint8Array,
  ): Promise<Uint8Array>;
  disconnect(): Promise<void>;
}

/** BIP-44 path for Stellar: m/44'/148'/<account>'. */
export function stellarDerivationPath(accountIndex: number): string {
  if (!Number.isInteger(accountIndex) || accountIndex < 0) {
    throw new RangeError('Account index must be a non-negative integer');
  }
  return `44'/148'/${accountIndex}'`;
}

/** Paths offered in the picker. */
export function defaultDerivationPaths(count = 5): string[] {
  return Array.from({ length: count }, (_, i) => stellarDerivationPath(i));
}

/**
 * WebHID is only exposed on secure origins, and only in Chromium. Checking up
 * front lets the UI say why the feature is unavailable instead of failing at
 * the moment the user clicks Connect.
 */
export function isWebHidSupported(
  nav: Navigator | undefined = typeof navigator === 'undefined'
    ? undefined
    : navigator,
): boolean {
  return Boolean(nav && 'hid' in nav);
}

/**
 * Translate a device error into something a user can act on.
 *
 * Ledger surfaces failures as status words on an error message; the raw text
 * ("0x6511") is meaningless to anyone who is not reading the APDU spec, and
 * showing it is how a recoverable "open the Stellar app" turns into a support
 * ticket.
 */
export function describeError(error: unknown): HardwareError {
  const message =
    error instanceof Error ? error.message : String(error ?? 'Unknown error');
  const lower = message.toLowerCase();

  if (lower.includes('6511') || lower.includes('app does not seem to be open')) {
    return {
      code: 'app-not-open',
      message,
      remedy: 'Open the Stellar app on your device, then try again.',
    };
  }
  if (lower.includes('6982') || lower.includes('locked')) {
    return {
      code: 'device-locked',
      message,
      remedy: 'Unlock your device with its PIN, then try again.',
    };
  }
  if (
    lower.includes('6985') ||
    lower.includes('denied') ||
    lower.includes('refused') ||
    lower.includes('rejected')
  ) {
    return {
      code: 'user-rejected',
      message,
      remedy: 'The request was declined on the device. Approve it to continue.',
    };
  }
  if (lower.includes('no device selected') || lower.includes('no device found')) {
    return {
      code: 'no-device',
      message,
      remedy: 'Plug in your device and select it in the browser prompt.',
    };
  }
  if (lower.includes('transport') || lower.includes('disconnected')) {
    return {
      code: 'transport',
      message,
      remedy: 'Reconnect the device and try again.',
    };
  }
  return {
    code: 'unknown',
    message,
    remedy: 'Reconnect the device and try again.',
  };
}

/**
 * Ledger adapter over WebHID.
 *
 * The transport and app modules are imported dynamically so that the Ledger
 * bundle — which is large and touches browser-only APIs — is not pulled into
 * the main chunk, or into a test run that never talks to a device.
 */
export function createLedgerAdapter(): HardwareWalletAdapter {
  let transport: { close(): Promise<void> } | null = null;
  // Structural, not imported from the Ledger package: importing its types
  // statically would defeat the dynamic import below and pull the bundle into
  // the main chunk.
  let app: {
    getPublicKey(path: string): Promise<{
      publicKey?: string;
      rawPublicKey?: Uint8Array;
    }>;
    signTransaction(
      path: string,
      tx: Uint8Array,
    ): Promise<{ signature: Uint8Array }>;
  } | null = null;

  return {
    vendor: 'ledger',

    async connect() {
      if (!isWebHidSupported()) {
        throw new Error('WebHID is not available in this browser');
      }
      const [{ default: TransportWebHID }, Str] = await Promise.all([
        import('@ledgerhq/hw-transport-webhid'),
        import('@ledgerhq/hw-app-str'),
      ]);
      transport = await TransportWebHID.create();
      const StrApp = (Str as { default: new (t: unknown) => typeof app }).default;
      app = new StrApp(transport) as typeof app;
    },

    async getPublicKey(derivationPath: string) {
      if (!app) throw new Error('Transport is not connected');
      const result = await app.getPublicKey(derivationPath);
      // Older app versions return an encoded `publicKey`, newer ones the raw
      // 32 bytes; accept either rather than pinning one app version.
      if (result.publicKey) return result.publicKey;
      if (result.rawPublicKey) return encodeRawKey(result.rawPublicKey);
      throw new Error('Device returned no public key');
    },

    async signTransaction(derivationPath: string, payload: Uint8Array) {
      if (!app) throw new Error('Transport is not connected');
      const result = await app.signTransaction(derivationPath, payload);
      return new Uint8Array(result.signature);
    },

    async disconnect() {
      await transport?.close();
      transport = null;
      app = null;
    },
  };
}

/** Raw 32-byte key → StrKey `G…` address. */
function encodeRawKey(raw: ArrayLike<number>): string {
  const payload = new Uint8Array(raw.length + 3);
  payload[0] = 6 << 3; // version byte for an ed25519 public key
  payload.set(raw, 1);

  const checksum = crc16Xmodem(payload.subarray(0, raw.length + 1));
  payload[raw.length + 1] = checksum & 0xff;
  payload[raw.length + 2] = (checksum >> 8) & 0xff;

  return base32Encode(payload);
}

function crc16Xmodem(bytes: Uint8Array): number {
  let crc = 0;
  for (const byte of bytes) {
    crc ^= byte << 8;
    for (let i = 0; i < 8; i++) {
      crc = crc & 0x8000 ? ((crc << 1) ^ 0x1021) & 0xffff : (crc << 1) & 0xffff;
    }
  }
  return crc;
}

const BASE32_ALPHABET = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567';

function base32Encode(bytes: Uint8Array): string {
  let bits = 0;
  let value = 0;
  let output = '';
  for (const byte of bytes) {
    value = (value << 8) | byte;
    bits += 8;
    while (bits >= 5) {
      output += BASE32_ALPHABET[(value >>> (bits - 5)) & 31];
      bits -= 5;
    }
  }
  if (bits > 0) {
    output += BASE32_ALPHABET[(value << (5 - bits)) & 31];
  }
  return output;
}
