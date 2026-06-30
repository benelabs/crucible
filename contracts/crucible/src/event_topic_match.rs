// Shared helpers for matching Soroban event topics.
//
// NOTE: This module exists to ensure all public event-filtering helpers use the
// same topic comparison strategy. Comparison is done via the host environment's
// structural equality (`Env::compare`), not raw payload bits, since object-backed
// values (String, Bytes, Vec, Map, Address, structs/enums) are represented as
// handles into the host's object store — two semantically-equal values can have
// different payloads, so `Val::get_payload()` comparison silently fails to match.

use soroban_env_host::Compare;
use soroban_sdk::{Env, Val, Vec as SorobanVec};

pub(crate) fn topics_match(
    env: &Env,
    filter_topics: &SorobanVec<Val>,
    event_topics: &SorobanVec<Val>,
) -> bool {
    if event_topics.len() < filter_topics.len() {
        return false;
    }

    filter_topics.iter().enumerate().all(|(i, filter_topic)| {
        let ev_topic = event_topics.get(i as u32).unwrap();
        env.compare(&filter_topic, &ev_topic) == Ok(core::cmp::Ordering::Equal)
    })
}
