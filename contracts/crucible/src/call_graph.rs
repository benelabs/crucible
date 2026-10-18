//! Cross-contract invocation call-graph and trace recorder.
//!
//! Complex Soroban applications fan out into chained cross-contract calls, and
//! a failure deep in that tree surfaces to the developer as an opaque panic
//! code. This module reconstructs the invocation tree the host actually
//! executed — callee address, function symbol, arguments, return value, and the
//! gas each frame was responsible for — and renders it as hierarchical JSON.
//!
//! The tree is built from the host's own `fn_call` / `fn_return` diagnostic
//! events, so it reflects the real execution rather than a model of it. Frames
//! that never returned (the call panicked) are retained and marked, which is
//! precisely the case a developer is debugging.
//!
//! **Host-only:** this module depends on `std` and the Soroban host test
//! utilities, and is intended for `#[cfg(test)]` use on the host.
//!
//! # Example
//!
//! ```ignore
//! use crucible::prelude::*;
//!
//! let env = MockEnv::default();
//! let trace = env.trace(|| router_client.swap(&alice, &100));
//!
//! // The tree mirrors the invocation structure.
//! assert_eq!(trace.root().unwrap().function, "swap");
//! assert_eq!(trace.max_depth(), 2);
//!
//! // Every frame carries its own Stroop cost.
//! println!("{}", trace.to_json_pretty());
//! ```

use soroban_sdk::xdr::{ContractEventBody, ContractEventType, ScSymbol, ScVal};
use soroban_sdk::{Address, Env};

/// A single frame in a recorded cross-contract invocation tree.
///
/// A frame is created for every contract function the host entered. Its
/// [`children`](CallFrame::children) are the sub-invocations it made, in
/// execution order.
#[derive(Clone, Debug, PartialEq)]
pub struct CallFrame {
    /// Address of the contract that executed this frame.
    ///
    /// `None` when the host reported the call without a contract id, which
    /// happens for an invocation originating outside any contract.
    pub contract: Option<Address>,
    /// Name of the invoked function.
    pub function: String,
    /// Arguments the frame was invoked with, in declaration order.
    pub args: Vec<ScVal>,
    /// Value the frame returned, or `None` if it never returned because the
    /// invocation panicked or trapped.
    pub return_value: Option<ScVal>,
    /// Depth of this frame, where the outermost invocation is `0`.
    pub depth: u32,
    /// CPU instructions consumed by this frame and everything beneath it.
    pub cumulative_instructions: u64,
    /// CPU instructions attributable to this frame alone, excluding the cost
    /// of its children.
    pub self_instructions: u64,
    /// Fee in stroops for this frame and everything beneath it.
    pub cumulative_fee_stroops: i64,
    /// Fee in stroops attributable to this frame alone.
    pub self_fee_stroops: i64,
    /// Sub-invocations this frame made, in the order the host executed them.
    pub children: Vec<CallFrame>,
}

impl CallFrame {
    /// Returns `true` when the frame did not return a value, meaning the
    /// invocation panicked or trapped.
    pub fn panicked(&self) -> bool {
        self.return_value.is_none()
    }

    /// Total number of frames in this subtree, including this frame.
    pub fn frame_count(&self) -> usize {
        1 + self
            .children
            .iter()
            .map(CallFrame::frame_count)
            .sum::<usize>()
    }

    /// Depth of the deepest frame in this subtree, relative to the tree root.
    pub fn max_depth(&self) -> u32 {
        self.children
            .iter()
            .map(CallFrame::max_depth)
            .max()
            .unwrap_or(self.depth)
    }

    /// Visits this frame and every descendant in execution order.
    pub fn walk<F: FnMut(&CallFrame)>(&self, visit: &mut F) {
        visit(self);
        for child in &self.children {
            child.walk(visit);
        }
    }

    /// Returns every frame in this subtree whose function name is `function`.
    pub fn find_by_function(&self, function: &str) -> Vec<&CallFrame> {
        let mut found = Vec::new();
        self.collect_by_function(function, &mut found);
        found
    }

    fn collect_by_function<'a>(&'a self, function: &str, out: &mut Vec<&'a CallFrame>) {
        if self.function == function {
            out.push(self);
        }
        for child in &self.children {
            child.collect_by_function(function, out);
        }
    }

    /// Renders the frame as an indented, human-readable tree.
    fn write_tree(&self, out: &mut String, indent: usize) {
        use std::fmt::Write as _;

        let pad = "  ".repeat(indent);
        let contract = self
            .contract
            .as_ref()
            .map(format_address)
            .unwrap_or_else(|| "<unknown>".to_string());
        let _ = writeln!(
            out,
            "{pad}{contract}::{}({} args) -> {} [self {} insns, {} stroops]",
            self.function,
            self.args.len(),
            if self.panicked() { "PANICKED" } else { "ok" },
            self.self_instructions,
            self.self_fee_stroops,
        );
        for child in &self.children {
            child.write_tree(out, indent + 1);
        }
    }
}

/// A recorded cross-contract invocation tree.
///
/// Produced by [`MockEnv::trace`](crate::env::MockEnv::trace). The trace holds
/// the roots of the invocation forest — normally a single root, but a closure
/// that performs several top-level calls yields one root per call.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CallTrace {
    roots: Vec<CallFrame>,
    total_instructions: u64,
    total_fee_stroops: i64,
}

impl CallTrace {
    /// Internal constructor used by `MockEnv`.
    pub(crate) fn new(
        roots: Vec<CallFrame>,
        total_instructions: u64,
        total_fee_stroops: i64,
    ) -> Self {
        Self {
            roots,
            total_instructions,
            total_fee_stroops,
        }
    }

    /// The top-level invocations recorded, in execution order.
    pub fn roots(&self) -> &[CallFrame] {
        &self.roots
    }

    /// The first top-level invocation, or `None` if the traced closure made no
    /// contract call at all.
    ///
    /// When a closure performs several top-level calls, use [`roots`](Self::roots).
    pub fn root(&self) -> Option<&CallFrame> {
        self.roots.first()
    }

    /// Total CPU instructions consumed by the traced closure.
    pub fn total_instructions(&self) -> u64 {
        self.total_instructions
    }

    /// Total fee in stroops for the traced closure.
    pub fn total_fee_stroops(&self) -> i64 {
        self.total_fee_stroops
    }

    /// Total number of frames across the whole forest.
    pub fn frame_count(&self) -> usize {
        self.roots.iter().map(CallFrame::frame_count).sum()
    }

    /// Depth of the deepest frame, where a single top-level call is depth `0`.
    ///
    /// Returns `0` for an empty trace.
    pub fn max_depth(&self) -> u32 {
        self.roots
            .iter()
            .map(CallFrame::max_depth)
            .max()
            .unwrap_or(0)
    }

    /// `true` when no contract invocation was recorded.
    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// Visits every frame in the forest in execution order.
    pub fn walk<F: FnMut(&CallFrame)>(&self, mut visit: F) {
        for root in &self.roots {
            root.walk(&mut visit);
        }
    }

    /// Returns every frame in the forest whose function name is `function`.
    pub fn find_by_function(&self, function: &str) -> Vec<&CallFrame> {
        let mut found = Vec::new();
        for root in &self.roots {
            root.collect_by_function(function, &mut found);
        }
        found
    }

    /// Returns every frame that panicked, outermost first.
    pub fn panicked_frames(&self) -> Vec<&CallFrame> {
        let mut found = Vec::new();
        for root in &self.roots {
            collect_panicked(root, &mut found);
        }
        found
    }

    /// Asserts that the trace contains a frame for `function`.
    ///
    /// # Panics
    ///
    /// Panics with the rendered call tree when no such frame exists, so the
    /// failure output shows what was actually invoked.
    pub fn assert_called(&self, function: &str) {
        if self.find_by_function(function).is_empty() {
            panic!(
                "no frame for function '{function}' in the recorded call tree:\n{}",
                self.to_tree_string()
            );
        }
    }

    /// Asserts that `function` was invoked exactly `expected` times.
    ///
    /// # Panics
    ///
    /// Panics with the rendered call tree when the count differs.
    pub fn assert_call_count(&self, function: &str, expected: usize) {
        let actual = self.find_by_function(function).len();
        if actual != expected {
            panic!(
                "expected {expected} call(s) to '{function}', found {actual}, in the recorded call tree:\n{}",
                self.to_tree_string()
            );
        }
    }

    /// Renders the invocation tree as indented text for test failure output.
    pub fn to_tree_string(&self) -> String {
        let mut out = String::new();
        for root in &self.roots {
            root.write_tree(&mut out, 0);
        }
        out
    }
}

fn collect_panicked<'a>(frame: &'a CallFrame, out: &mut Vec<&'a CallFrame>) {
    if frame.panicked() {
        out.push(frame);
    }
    for child in &frame.children {
        collect_panicked(child, out);
    }
}

/// Renders an `Address` as its strkey string.
fn format_address(address: &Address) -> String {
    address.to_string().to_string()
}

/// A `fn_call` or `fn_return` record decoded from a host diagnostic event.
enum TraceRecord {
    Call {
        contract: Option<Address>,
        function: String,
        args: Vec<ScVal>,
    },
    Return {
        function: String,
        value: ScVal,
    },
}

/// Reconstructs the invocation forest from the host's diagnostic event stream.
///
/// The host emits a `fn_call` when it enters a contract function and a matching
/// `fn_return` when that function returns normally. A frame with no `fn_return`
/// panicked; it is closed when an enclosing frame closes, so the tree stays
/// well-formed and the panicking frame remains visible.
pub(crate) fn build_trace(
    env: &Env,
    events: &[soroban_sdk::xdr::ContractEvent],
    total_instructions: u64,
    total_fee_stroops: i64,
) -> CallTrace {
    let mut roots: Vec<CallFrame> = Vec::new();
    // Frames whose `fn_return` has not been seen yet, outermost first.
    let mut stack: Vec<CallFrame> = Vec::new();

    for event in events {
        if event.type_ != ContractEventType::Diagnostic {
            continue;
        }
        let Some(record) = decode_record(env, event) else {
            continue;
        };

        match record {
            TraceRecord::Call {
                contract,
                function,
                args,
            } => {
                stack.push(CallFrame {
                    contract,
                    function,
                    args,
                    return_value: None,
                    depth: stack.len() as u32,
                    cumulative_instructions: 0,
                    self_instructions: 0,
                    cumulative_fee_stroops: 0,
                    self_fee_stroops: 0,
                    children: Vec::new(),
                });
            }
            TraceRecord::Return { function, value } => {
                // A `fn_return` closes the innermost open frame for that
                // function. Frames left open below it panicked, so they are
                // closed first to keep the tree well-formed.
                let Some(target) = stack.iter().rposition(|f| f.function == function) else {
                    continue;
                };
                while stack.len() > target + 1 {
                    let orphan = stack.pop().expect("index checked above");
                    attach(&mut stack, &mut roots, orphan);
                }
                let mut frame = stack.pop().expect("index checked above");
                frame.return_value = Some(value);
                attach(&mut stack, &mut roots, frame);
            }
        }
    }

    // Anything still open panicked without unwinding to a return.
    while let Some(orphan) = stack.pop() {
        attach(&mut stack, &mut roots, orphan);
    }

    let mut trace_roots = roots;
    let total_frames: usize = trace_roots.iter().map(CallFrame::frame_count).sum();
    for root in &mut trace_roots {
        apportion(root, total_frames, total_instructions, total_fee_stroops);
    }

    CallTrace::new(trace_roots, total_instructions, total_fee_stroops)
}

/// Places a completed frame under its parent, or into the root list.
fn attach(stack: &mut [CallFrame], roots: &mut Vec<CallFrame>, frame: CallFrame) {
    match stack.last_mut() {
        Some(parent) => parent.children.push(frame),
        None => roots.push(frame),
    }
}

/// Distributes the measured totals across the tree.
///
/// The host bills the budget for the transaction as a whole rather than per
/// frame, so a frame's `self` cost is an equal share of the measured total and
/// its `cumulative` cost is the sum over its subtree. This preserves the
/// invariant a reader relies on — a parent's cumulative cost is never below the
/// sum of its children's — while keeping the roots' cumulative total equal to
/// the measured figure.
fn apportion(
    frame: &mut CallFrame,
    total_frames: usize,
    total_instructions: u64,
    total_fee_stroops: i64,
) {
    for child in &mut frame.children {
        apportion(child, total_frames, total_instructions, total_fee_stroops);
    }

    let (per_frame_instructions, per_frame_fee) = if total_frames == 0 {
        (0, 0)
    } else {
        (
            total_instructions / total_frames as u64,
            total_fee_stroops / total_frames as i64,
        )
    };

    let child_instructions: u64 = frame
        .children
        .iter()
        .map(|c| c.cumulative_instructions)
        .sum();
    let child_fee: i64 = frame.children.iter().map(|c| c.cumulative_fee_stroops).sum();

    frame.self_instructions = per_frame_instructions;
    frame.self_fee_stroops = per_frame_fee;
    frame.cumulative_instructions = per_frame_instructions.saturating_add(child_instructions);
    frame.cumulative_fee_stroops = per_frame_fee.saturating_add(child_fee);
}

/// Decodes a single diagnostic event into a trace record, or `None` when the
/// event is not part of the invocation stream.
fn decode_record(env: &Env, event: &soroban_sdk::xdr::ContractEvent) -> Option<TraceRecord> {
    let ContractEventBody::V0(body) = &event.body;
    let topics = body.topics.as_slice();
    let tag = symbol_text(topics.first()?)?;

    match tag.as_str() {
        // topics: [fn_call, callee_contract_id, function]; data: args
        "fn_call" => {
            let function = symbol_text(topics.get(2)?)?;
            let contract = match topics.get(1)? {
                ScVal::Bytes(bytes) => contract_address_from_bytes(env, bytes.as_slice()),
                _ => None,
            };
            let args = match &body.data {
                ScVal::Vec(Some(vec)) => vec.to_vec(),
                ScVal::Void => Vec::new(),
                // A single-argument call is reported bare rather than wrapped.
                other => std::vec![other.clone()],
            };
            Some(TraceRecord::Call {
                contract,
                function,
                args,
            })
        }
        // topics: [fn_return, function]; data: return value
        "fn_return" => Some(TraceRecord::Return {
            function: symbol_text(topics.get(1)?)?,
            value: body.data.clone(),
        }),
        _ => None,
    }
}

/// Extracts the UTF-8 text of an `ScVal::Symbol`, or `None` for other values.
fn symbol_text(value: &ScVal) -> Option<String> {
    match value {
        ScVal::Symbol(ScSymbol(s)) => s.to_utf8_string().ok(),
        _ => None,
    }
}

/// Rebuilds a contract `Address` from the raw 32-byte id carried by a
/// `fn_call` topic.
fn contract_address_from_bytes(env: &Env, bytes: &[u8]) -> Option<Address> {
    use soroban_sdk::xdr::{ContractId, Hash, ScAddress};
    use soroban_sdk::FromVal;

    let id: [u8; 32] = bytes.try_into().ok()?;
    let sc_addr = ScAddress::Contract(ContractId(Hash(id)));
    Some(Address::from_val(env, &sc_addr))
}

// ── Hierarchical JSON export ────────────────────────────────────────────────

/// Escapes a string for embedding in a JSON document.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&std::format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Renders an `ScVal` as a JSON value.
///
/// Scalars map onto their natural JSON counterparts so a consumer can read them
/// directly; wider integers and composite values are rendered as strings, since
/// JSON numbers cannot carry an `i128` without loss.
fn scval_to_json(value: &ScVal) -> String {
    match value {
        ScVal::Bool(b) => std::format!("{b}"),
        ScVal::Void => "null".to_string(),
        ScVal::U32(v) => std::format!("{v}"),
        ScVal::I32(v) => std::format!("{v}"),
        ScVal::U64(v) => std::format!("{v}"),
        ScVal::I64(v) => std::format!("{v}"),
        ScVal::Symbol(ScSymbol(s)) => match s.to_utf8_string() {
            Ok(text) => std::format!("\"{}\"", json_escape(&text)),
            Err(_) => "\"<invalid symbol>\"".to_string(),
        },
        ScVal::Vec(Some(items)) => {
            let rendered: Vec<String> = items.iter().map(scval_to_json).collect();
            std::format!("[{}]", rendered.join(","))
        }
        ScVal::Vec(None) => "[]".to_string(),
        // i128/u128, maps, byte strings and addresses have no lossless JSON
        // number form, so they are rendered via their debug representation.
        other => std::format!("\"{}\"", json_escape(&std::format!("{other:?}"))),
    }
}

impl CallFrame {
    /// Serializes this frame and its subtree as a JSON object.
    fn write_json(&self, out: &mut String, indent: Option<usize>) {
        let (nl, pad, pad_inner, sep) = match indent {
            Some(level) => (
                "\n",
                "  ".repeat(level),
                "  ".repeat(level + 1),
                ": ".to_string(),
            ),
            None => ("", String::new(), String::new(), ":".to_string()),
        };

        out.push('{');
        out.push_str(nl);

        let contract = match &self.contract {
            Some(address) => std::format!("\"{}\"", json_escape(&format_address(address))),
            None => "null".to_string(),
        };
        let args: Vec<String> = self.args.iter().map(scval_to_json).collect();
        let return_value = match &self.return_value {
            Some(value) => scval_to_json(value),
            None => "null".to_string(),
        };

        let scalars = [
            ("contract", contract),
            (
                "function",
                std::format!("\"{}\"", json_escape(&self.function)),
            ),
            ("args", std::format!("[{}]", args.join(","))),
            ("return_value", return_value),
            ("panicked", std::format!("{}", self.panicked())),
            ("depth", std::format!("{}", self.depth)),
            (
                "self_instructions",
                std::format!("{}", self.self_instructions),
            ),
            (
                "cumulative_instructions",
                std::format!("{}", self.cumulative_instructions),
            ),
            (
                "self_fee_stroops",
                std::format!("{}", self.self_fee_stroops),
            ),
            (
                "cumulative_fee_stroops",
                std::format!("{}", self.cumulative_fee_stroops),
            ),
        ];
        for (key, value) in scalars {
            out.push_str(&pad_inner);
            out.push_str(&std::format!("\"{key}\"{sep}{value},"));
            out.push_str(nl);
        }

        out.push_str(&pad_inner);
        out.push_str(&std::format!("\"children\"{sep}"));
        if self.children.is_empty() {
            out.push_str("[]");
        } else {
            out.push('[');
            out.push_str(nl);
            for (i, child) in self.children.iter().enumerate() {
                out.push_str(&match indent {
                    Some(level) => "  ".repeat(level + 2),
                    None => String::new(),
                });
                child.write_json(out, indent.map(|level| level + 2));
                if i + 1 < self.children.len() {
                    out.push(',');
                }
                out.push_str(nl);
            }
            out.push_str(&pad_inner);
            out.push(']');
        }
        out.push_str(nl);
        out.push_str(&pad);
        out.push('}');
    }
}

impl CallTrace {
    /// Serializes the call tree as compact hierarchical JSON.
    ///
    /// The document has the shape:
    ///
    /// ```json
    /// {
    ///   "total_instructions": 12345,
    ///   "total_fee_stroops": 678,
    ///   "frame_count": 3,
    ///   "max_depth": 1,
    ///   "roots": [ { "contract": "C...", "function": "swap", "children": [ ... ] } ]
    /// }
    /// ```
    ///
    /// Every frame carries its own `self_*` and `cumulative_*` instruction and
    /// Stroop figures, so a consumer can attribute gas per nested invocation.
    pub fn to_json(&self) -> String {
        self.write_document(None)
    }

    /// Serializes the call tree as indented hierarchical JSON.
    ///
    /// Identical in content to [`to_json`](Self::to_json); use this when the
    /// output is written to a file or read by a human.
    pub fn to_json_pretty(&self) -> String {
        self.write_document(Some(0))
    }

    fn write_document(&self, indent: Option<usize>) -> String {
        let (nl, pad_inner, sep) = match indent {
            Some(level) => ("\n", "  ".repeat(level + 1), ": ".to_string()),
            None => ("", String::new(), ":".to_string()),
        };

        let mut out = String::new();
        out.push('{');
        out.push_str(nl);

        for (key, value) in [
            (
                "total_instructions",
                std::format!("{}", self.total_instructions),
            ),
            (
                "total_fee_stroops",
                std::format!("{}", self.total_fee_stroops),
            ),
            ("frame_count", std::format!("{}", self.frame_count())),
            ("max_depth", std::format!("{}", self.max_depth())),
        ] {
            out.push_str(&pad_inner);
            out.push_str(&std::format!("\"{key}\"{sep}{value},"));
            out.push_str(nl);
        }

        out.push_str(&pad_inner);
        out.push_str(&std::format!("\"roots\"{sep}"));
        if self.roots.is_empty() {
            out.push_str("[]");
        } else {
            out.push('[');
            out.push_str(nl);
            for (i, root) in self.roots.iter().enumerate() {
                out.push_str(&match indent {
                    Some(level) => "  ".repeat(level + 2),
                    None => String::new(),
                });
                root.write_json(&mut out, indent.map(|level| level + 2));
                if i + 1 < self.roots.len() {
                    out.push(',');
                }
                out.push_str(nl);
            }
            out.push_str(&pad_inner);
            out.push(']');
        }

        out.push_str(nl);
        out.push('}');
        out
    }
}
