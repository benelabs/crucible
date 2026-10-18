//! Cross-contract invocation call-graph and trace recorder tests.
//!
//! Exercises the recorder against a three-tier protocol (router → pool →
//! ledger) so the assertions cover a genuinely nested invocation tree rather
//! than a single call.

use crucible::prelude::*;
use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ProtocolError {
    InsufficientLiquidity = 1,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Pool,
    Ledger,
    Liquidity,
    Recorded,
}

/// Innermost tier: records settled amounts.
#[contract]
struct Ledger;

#[contractimpl]
impl Ledger {
    pub fn record(env: Env, amount: i128) -> i128 {
        let total: i128 = env.storage().instance().get(&DataKey::Recorded).unwrap_or(0);
        let updated = total + amount;
        env.storage().instance().set(&DataKey::Recorded, &updated);
        updated
    }

    pub fn recorded(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::Recorded).unwrap_or(0)
    }
}

/// Middle tier: draws down liquidity, then settles through the ledger.
#[contract]
struct Pool;

#[contractimpl]
impl Pool {
    pub fn init(env: Env, ledger: Address, liquidity: i128) {
        env.storage().instance().set(&DataKey::Ledger, &ledger);
        env.storage().instance().set(&DataKey::Liquidity, &liquidity);
    }

    pub fn draw(env: Env, amount: i128) -> i128 {
        let available: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Liquidity)
            .unwrap_or(0);
        if available < amount {
            soroban_sdk::panic_with_error!(&env, ProtocolError::InsufficientLiquidity);
        }
        env.storage()
            .instance()
            .set(&DataKey::Liquidity, &(available - amount));

        let ledger: Address = env.storage().instance().get(&DataKey::Ledger).unwrap();
        LedgerClient::new(&env, &ledger).record(&amount)
    }
}

/// Outermost tier: the entry point a user calls.
#[contract]
struct Router;

#[contractimpl]
impl Router {
    pub fn init(env: Env, pool: Address) {
        env.storage().instance().set(&DataKey::Pool, &pool);
    }

    pub fn swap(env: Env, amount: i128) -> i128 {
        let pool: Address = env.storage().instance().get(&DataKey::Pool).unwrap();
        PoolClient::new(&env, &pool).draw(&amount)
    }

    /// Draws twice, so a single top-level call fans out into sibling subtrees.
    pub fn swap_twice(env: Env, amount: i128) -> i128 {
        let pool: Address = env.storage().instance().get(&DataKey::Pool).unwrap();
        let client = PoolClient::new(&env, &pool);
        client.draw(&amount);
        client.draw(&amount)
    }
}

struct Protocol {
    env: MockEnv,
    router: Address,
    pool: Address,
    ledger: Address,
}

fn deploy(liquidity: i128) -> Protocol {
    let env = MockEnv::default();
    env.mock_all_auths();
    let inner = env.inner();

    let ledger = inner.register(Ledger, ());
    let pool = inner.register(Pool, ());
    let router = inner.register(Router, ());

    PoolClient::new(inner, &pool).init(&ledger, &liquidity);
    RouterClient::new(inner, &router).init(&pool);

    Protocol {
        env,
        router,
        pool,
        ledger,
    }
}

#[test]
fn trace_records_the_full_invocation_tree() {
    let p = deploy(1_000);
    let client = RouterClient::new(p.env.inner(), &p.router);

    let (result, trace) = p.env.trace(|| client.swap(&250));

    assert_eq!(result, 250, "the traced call still returns its value");

    // router::swap -> pool::draw -> ledger::record
    assert_eq!(trace.frame_count(), 3);
    assert_eq!(trace.max_depth(), 2);

    let root = trace.root().expect("one top-level invocation");
    assert_eq!(root.function, "swap");
    assert_eq!(root.contract.as_ref(), Some(&p.router));
    assert_eq!(root.depth, 0);
    assert!(!root.panicked());

    let draw = &root.children[0];
    assert_eq!(draw.function, "draw");
    assert_eq!(draw.contract.as_ref(), Some(&p.pool));
    assert_eq!(draw.depth, 1);

    let record = &draw.children[0];
    assert_eq!(record.function, "record");
    assert_eq!(record.contract.as_ref(), Some(&p.ledger));
    assert_eq!(record.depth, 2);
    assert!(record.children.is_empty());
}

#[test]
fn trace_captures_arguments_and_return_values_per_frame() {
    use soroban_sdk::xdr::ScVal;

    let p = deploy(1_000);
    let client = RouterClient::new(p.env.inner(), &p.router);

    let (_, trace) = p.env.trace(|| client.swap(&250));

    let record = trace
        .find_by_function("record")
        .first()
        .copied()
        .expect("the ledger frame is recorded")
        .clone();

    assert_eq!(record.args.len(), 1, "record takes a single amount argument");
    let ScVal::I128(ref amount) = record.args[0] else {
        panic!("expected an i128 argument, got {:?}", record.args[0]);
    };
    assert_eq!(amount.lo, 250);

    let ScVal::I128(ref returned) = record
        .return_value
        .as_ref()
        .expect("a successful frame returns a value")
    else {
        panic!("expected an i128 return value");
    };
    assert_eq!(returned.lo, 250);
}

#[test]
fn trace_records_sibling_subtrees_in_execution_order() {
    let p = deploy(1_000);
    let client = RouterClient::new(p.env.inner(), &p.router);

    let (_, trace) = p.env.trace(|| client.swap_twice(&100));

    let root = trace.root().expect("one top-level invocation");
    assert_eq!(root.function, "swap_twice");
    assert_eq!(root.children.len(), 2, "two sibling draws");

    trace.assert_call_count("draw", 2);
    trace.assert_call_count("record", 2);
    // swap_twice + 2 draws + 2 records
    assert_eq!(trace.frame_count(), 5);
    assert_eq!(trace.max_depth(), 2);
}

#[test]
fn try_trace_reports_the_frame_that_panicked() {
    let p = deploy(100);
    let client = RouterClient::new(p.env.inner(), &p.router);

    // Draws more than the pool holds, so `draw` panics before reaching the ledger.
    let (result, trace) = p.env.try_trace(|| client.swap(&500));

    assert!(result.is_err(), "the swap must fail");

    let panicked = trace.panicked_frames();
    assert_eq!(
        panicked.len(),
        2,
        "the failing frame and its caller both fail to return: {}",
        trace.to_tree_string()
    );
    assert_eq!(panicked[0].function, "swap");
    assert_eq!(panicked[1].function, "draw");

    // The ledger was never reached, so no frame exists for it.
    assert!(trace.find_by_function("record").is_empty());
}

#[test]
fn trace_apportions_gas_across_the_tree() {
    let p = deploy(1_000);
    let client = RouterClient::new(p.env.inner(), &p.router);

    let (_, trace) = p.env.trace(|| client.swap(&250));

    assert!(
        trace.total_instructions() > 0,
        "a real invocation consumes instructions"
    );

    let root = trace.root().expect("one top-level invocation");

    // A parent's cumulative cost always covers its children's.
    trace.walk(|frame| {
        let child_sum: u64 = frame
            .children
            .iter()
            .map(|c| c.cumulative_instructions)
            .sum();
        assert!(
            frame.cumulative_instructions >= child_sum,
            "frame '{}' reports less cumulative gas than its children",
            frame.function
        );
        assert_eq!(
            frame.cumulative_instructions,
            frame.self_instructions + child_sum,
            "cumulative gas must be self plus children for '{}'",
            frame.function
        );
    });

    // The root accounts for the whole tree, up to integer division remainder.
    assert!(root.cumulative_instructions <= trace.total_instructions());
}

#[test]
fn trace_of_a_call_free_closure_is_empty() {
    let p = deploy(1_000);

    let (value, trace) = p.env.trace(|| 7_u32);

    assert_eq!(value, 7);
    assert!(trace.is_empty());
    assert_eq!(trace.frame_count(), 0);
    assert_eq!(trace.max_depth(), 0);
    assert!(trace.root().is_none());
}

#[test]
fn traces_are_scoped_to_their_own_closure() {
    let p = deploy(1_000);
    let client = RouterClient::new(p.env.inner(), &p.router);

    let (_, first) = p.env.trace(|| client.swap(&100));
    let (_, second) = p.env.trace(|| client.swap(&100));

    // The host appends to one event buffer for the lifetime of the
    // environment; a trace must still see only its own frames.
    assert_eq!(first.frame_count(), 3);
    assert_eq!(second.frame_count(), 3);
}

#[test]
fn assert_called_names_the_missing_function() {
    let p = deploy(1_000);
    let client = RouterClient::new(p.env.inner(), &p.router);

    let (_, trace) = p.env.trace(|| client.swap(&250));

    trace.assert_called("swap");
    trace.assert_called("draw");
    trace.assert_called("record");

    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        trace.assert_called("liquidate");
    }))
    .expect_err("asserting an uncalled function must panic");
    let message = failure
        .downcast_ref::<String>()
        .expect("panic carries a message");
    assert!(message.contains("liquidate"), "message was: {message}");
    assert!(
        message.contains("swap"),
        "the failure must show the recorded tree, was: {message}"
    );
}

#[test]
fn tree_string_renders_the_hierarchy() {
    let p = deploy(1_000);
    let client = RouterClient::new(p.env.inner(), &p.router);

    let (_, trace) = p.env.trace(|| client.swap(&250));
    let rendered = trace.to_tree_string();
    let lines: Vec<&str> = rendered.lines().collect();

    assert_eq!(lines.len(), 3);
    assert!(lines[0].starts_with('C'), "root is unindented: {}", lines[0]);
    assert!(lines[0].contains("::swap("));
    assert!(lines[1].starts_with("  "), "depth 1 is indented once");
    assert!(lines[1].contains("::draw("));
    assert!(lines[2].starts_with("    "), "depth 2 is indented twice");
    assert!(lines[2].contains("::record("));
}

// ── JSON export ─────────────────────────────────────────────────────────────

/// Minimal structural checks over the exported document.
///
/// The trace embeds live contract addresses and measured gas figures, neither
/// of which is stable across runs, so the snapshot assertions below pin the
/// document's structure rather than a byte-for-byte rendering.
#[test]
fn json_export_is_hierarchical_and_carries_per_frame_gas() {
    let p = deploy(1_000);
    let client = RouterClient::new(p.env.inner(), &p.router);

    let (_, trace) = p.env.trace(|| client.swap(&250));
    let json = trace.to_json();

    for key in [
        "\"total_instructions\":",
        "\"total_fee_stroops\":",
        "\"frame_count\":3",
        "\"max_depth\":2",
        "\"roots\":[",
    ] {
        assert!(json.contains(key), "missing {key} in: {json}");
    }

    // Each frame carries its own Stroop and instruction breakdown.
    assert_eq!(json.matches("\"self_fee_stroops\":").count(), 3);
    assert_eq!(json.matches("\"cumulative_fee_stroops\":").count(), 3);
    assert_eq!(json.matches("\"self_instructions\":").count(), 3);
    assert_eq!(json.matches("\"cumulative_instructions\":").count(), 3);

    // Nesting is expressed through `children`, one array per frame.
    assert_eq!(json.matches("\"children\":").count(), 3);
    assert!(json.contains("\"function\":\"swap\""));
    assert!(json.contains("\"function\":\"draw\""));
    assert!(json.contains("\"function\":\"record\""));
    assert!(json.contains("\"depth\":0"));
    assert!(json.contains("\"depth\":1"));
    assert!(json.contains("\"depth\":2"));
    assert!(json.contains("\"panicked\":false"));
}

#[test]
fn json_export_marks_the_panicking_frame() {
    let p = deploy(100);
    let client = RouterClient::new(p.env.inner(), &p.router);

    let (_, trace) = p.env.try_trace(|| client.swap(&500));
    let json = trace.to_json();

    assert!(json.contains("\"panicked\":true"));
    assert!(json.contains("\"return_value\":null"));
    assert!(!json.contains("\"function\":\"record\""));
}

#[test]
fn pretty_json_is_indented_and_matches_compact_content() {
    let p = deploy(1_000);
    let client = RouterClient::new(p.env.inner(), &p.router);

    let (_, trace) = p.env.trace(|| client.swap(&250));

    let pretty = trace.to_json_pretty();
    assert!(pretty.contains("\n"), "pretty output is multi-line");
    assert!(pretty.contains("  \"total_instructions\": "));

    // Stripping the pretty printer's whitespace yields the compact document.
    let collapsed: String = pretty
        .lines()
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("")
        .replace("\": ", "\":");
    assert_eq!(collapsed, trace.to_json());
}

#[test]
fn json_export_of_an_empty_trace_is_well_formed() {
    let p = deploy(1_000);

    let (_, trace) = p.env.trace(|| ());
    let json = trace.to_json();

    assert_eq!(
        json,
        "{\"total_instructions\":0,\"total_fee_stroops\":0,\"frame_count\":0,\"max_depth\":0,\"roots\":[]}"
    );
}

// ── Snapshot test ───────────────────────────────────────────────────────────

/// Normalizes a trace's JSON so it can be compared against a stored snapshot.
///
/// Contract addresses are allocation-order dependent and gas figures move with
/// the host version, so both are replaced with placeholders. What remains — the
/// tree shape, function names, depths, arguments and return values — is the
/// part a regression would break.
fn normalize(json: &str) -> String {
    let mut out = String::new();
    let mut rest = json;

    let numeric_keys = [
        "\"total_instructions\":",
        "\"total_fee_stroops\":",
        "\"self_instructions\":",
        "\"cumulative_instructions\":",
        "\"self_fee_stroops\":",
        "\"cumulative_fee_stroops\":",
    ];

    'outer: while !rest.is_empty() {
        for key in numeric_keys {
            if let Some(stripped) = rest.strip_prefix(key) {
                out.push_str(key);
                out.push_str("<n>");
                let end = stripped
                    .find(|c: char| c != '-' && !c.is_ascii_digit())
                    .unwrap_or(stripped.len());
                rest = &stripped[end..];
                continue 'outer;
            }
        }
        if let Some(stripped) = rest.strip_prefix("\"contract\":\"") {
            out.push_str("\"contract\":\"<address>");
            let end = stripped.find('"').unwrap_or(stripped.len());
            rest = &stripped[end..];
            continue;
        }
        let mut chars = rest.chars();
        let c = chars.next().expect("non-empty");
        out.push(c);
        rest = chars.as_str();
    }
    out
}

#[test]
fn snapshot_of_the_normalized_call_tree() {
    let p = deploy(1_000);
    let client = RouterClient::new(p.env.inner(), &p.router);

    let (_, trace) = p.env.trace(|| client.swap(&250));

    let expected = concat!(
        "{",
        "\"total_instructions\":<n>,",
        "\"total_fee_stroops\":<n>,",
        "\"frame_count\":3,",
        "\"max_depth\":2,",
        "\"roots\":[{",
        "\"contract\":\"<address>\",",
        "\"function\":\"swap\",",
        "\"args\":[\"I128(Int128Parts { hi: 0, lo: 250 })\"],",
        "\"return_value\":\"I128(Int128Parts { hi: 0, lo: 250 })\",",
        "\"panicked\":false,",
        "\"depth\":0,",
        "\"self_instructions\":<n>,",
        "\"cumulative_instructions\":<n>,",
        "\"self_fee_stroops\":<n>,",
        "\"cumulative_fee_stroops\":<n>,",
        "\"children\":[{",
        "\"contract\":\"<address>\",",
        "\"function\":\"draw\",",
        "\"args\":[\"I128(Int128Parts { hi: 0, lo: 250 })\"],",
        "\"return_value\":\"I128(Int128Parts { hi: 0, lo: 250 })\",",
        "\"panicked\":false,",
        "\"depth\":1,",
        "\"self_instructions\":<n>,",
        "\"cumulative_instructions\":<n>,",
        "\"self_fee_stroops\":<n>,",
        "\"cumulative_fee_stroops\":<n>,",
        "\"children\":[{",
        "\"contract\":\"<address>\",",
        "\"function\":\"record\",",
        "\"args\":[\"I128(Int128Parts { hi: 0, lo: 250 })\"],",
        "\"return_value\":\"I128(Int128Parts { hi: 0, lo: 250 })\",",
        "\"panicked\":false,",
        "\"depth\":2,",
        "\"self_instructions\":<n>,",
        "\"cumulative_instructions\":<n>,",
        "\"self_fee_stroops\":<n>,",
        "\"cumulative_fee_stroops\":<n>,",
        "\"children\":[]",
        "}]",
        "}]",
        "}]",
        "}",
    );

    assert_eq!(normalize(&trace.to_json()), expected);
}
