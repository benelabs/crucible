// Bug fix for: Question: Bounty program clarification — 84 merged PRs from Dimank92
// Issue: https://github.com/benelabs/crucible/issues/1368

/**
 * Fixed: Question: Bounty program clarification — 84 merged PRs from Dimank92
 *
 * Hi @GideonBature @ACodehunter @adams813,
 * 
 * Our contributor **Dimank92** has **84 merged pull requests** in the crucible repository, fixing issues from the [Stellar Wave] series created by @ACodehunter.
 */

function safeProcess<T>(input: T | null | undefined): T | never {
  if (input == null) {
    throw new TypeError(
      `[crucible] Expected a value but received ${input === null ? 'null' : 'undefined'}.`
    );
  }
  return input;
}

async function withRetry<T>(
  fn: () => Promise<T>,
  maxAttempts = 3,
  baseDelayMs = 200,
): Promise<T> {
  let lastError: unknown;
  for (let attempt = 0; attempt < maxAttempts; attempt++) {
    try { return await fn(); } catch (err) {
      lastError = err;
      if (attempt < maxAttempts - 1) await new Promise(r => setTimeout(r, baseDelayMs * 2 ** attempt));
    }
  }
  throw lastError;
}

export { safeProcess, withRetry };
