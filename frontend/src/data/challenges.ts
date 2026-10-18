import type { Challenge } from '../components/InteractiveChallengeEngine';

export const COUNTER_CHALLENGE: Challenge = {
  id: 'soroban-counter',
  title: 'Build a Soroban Counter Contract',
  description: 'Fix the broken counter contract below so it compiles and persists its count in storage.',
  difficulty: 'beginner',
  initialCode: `#![no_std]
use soroban_sdk::{contract, contractimpl, Env};

#[contract]
pub struct CounterContract;

#[contractimpl]
impl CounterContract {
    // TODO: implement increment()
}`,
  testCases: [
    {
      name: 'Contract exports increment function',
      description: 'The contract must expose a public increment() function.',
      input: 'pub fn increment',
      expected: 'pub fn increment(env: Env) -> u32'
    },
    {
      name: 'Contract persists state in storage',
      description: 'The count must be read from and written to instance storage.',
      input: 'env.storage',
      expected: 'Counter value persisted across invocations'
    }
  ],
  hints: [
    'Every Soroban contract needs the #[contract] and #[contractimpl] macros from soroban_sdk.',
    'Use env.storage().instance().get(&key) and .set(&key, &value) to persist the counter.',
    'Return the updated count as a u32 from increment().'
  ],
  steps: [
    {
      id: 'step-struct',
      title: 'Define the contract struct',
      description: 'Add an increment() function to CounterContract that returns u32.',
      testCaseIndex: 0,
      hint: 'Use #[contract] on the struct and #[contractimpl] on the impl block.'
    },
    {
      id: 'step-storage',
      title: 'Persist the count',
      description: 'Store and update the counter using env.storage().instance().',
      testCaseIndex: 1,
      hint: 'Call env.storage().instance().get(&"count").unwrap_or(0) to read the current value.'
    }
  ]
};

export const CHALLENGES: Challenge[] = [COUNTER_CHALLENGE];
