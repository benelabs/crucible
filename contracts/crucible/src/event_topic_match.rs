// Shared helpers for matching Soroban event topics.
//
// NOTE: This module exists to ensure all public event-filtering helpers use the
// same topic comparison strategy. Comparison is done via the host environment's
// structural equality (`Env::compare`), not raw payload bits, since object-backed
// values (String, Bytes, Vec, Map, Address, structs/enums) are represented as
// handles into the host's object store — two semantically-equal values can have
// different payloads, so `Val::get_payload()` comparison silently fails to match.

use soroban_env_host::Compare;
use soroban_sdk::{symbol_short, Env, Symbol, TryFromVal, Val};
use soroban_sdk::Vec as SorobanVec;

pub(crate) fn is_wildcard_topic(env: &Env, filter_topic: &Val) -> bool {
    if filter_topic.is_void() {
        return true;
    }
    if let Ok(sym) = Symbol::try_from_val(env, filter_topic) {
        let star = Symbol::new(env, "*");
        let underscore = symbol_short!("_");
        if env.compare(&sym.to_val(), &star.to_val()) == Ok(core::cmp::Ordering::Equal)
            || env.compare(&sym.to_val(), &underscore.to_val()) == Ok(core::cmp::Ordering::Equal)
        {
            return true;
        }
    }
    false
}

pub(crate) fn topic_segment_matches(env: &Env, filter_topic: &Val, ev_topic: &Val) -> bool {
    if is_wildcard_topic(env, filter_topic) {
        true
    } else {
        env.compare(filter_topic, ev_topic) == Ok(core::cmp::Ordering::Equal)
    }
}

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
        topic_segment_matches(env, &filter_topic, &ev_topic)
    })
}
