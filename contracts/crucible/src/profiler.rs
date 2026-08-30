//! Gas profiler and flamegraph exporter for Soroban contract testing.
//!
//! This module provides tools to profile contract invocations, generate
//! speedscope-compatible JSON, and export SVG flamegraphs for visual analysis
//! of gas consumption and host function overheads.
//!
//! **Host-only:** All types in this module depend on `std` and are intended
//! exclusively for use in `#[cfg(test)]` contexts on the host.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// A single stack frame in a call profile.
#[derive(Debug, Clone)]
pub struct Frame {
    pub name: String,
    pub function: String,
    pub file: Option<String>,
    pub line: Option<u32>,
}

impl Frame {
    pub fn new(name: impl Into<String>, function: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            function: function.into(),
            file: None,
            line: None,
        }
    }

    pub fn with_location(mut self, file: impl Into<String>, line: u32) -> Self {
        self.file = Some(file.into());
        self.line = Some(line);
        self
    }
}

/// A single sample in a profile, recording a stack and its cost.
#[derive(Debug, Clone)]
pub struct Sample {
    pub stack: Vec<Frame>,
    pub cost: u64,
    pub memory_bytes: u64,
}

impl Sample {
    pub fn new(stack: Vec<Frame>, cost: u64, memory_bytes: u64) -> Self {
        Self {
            stack,
            cost,
            memory_bytes,
        }
    }
}

/// A complete profile recording for a single contract invocation.
#[derive(Debug, Clone, Default)]
pub struct Profile {
    pub samples: Vec<Sample>,
    pub start_time: u64,
    pub end_time: u64,
}

impl Profile {
    pub fn new() -> Self {
        let now = timestamp_ms();
        Self {
            samples: Vec::new(),
            start_time: now,
            end_time: now,
        }
    }

    pub fn add_sample(&mut self, sample: Sample) {
        self.samples.push(sample);
    }

    pub fn finish(&mut self) {
        self.end_time = timestamp_ms();
    }

    pub fn total_cost(&self) -> u64 {
        self.samples.iter().map(|s| s.cost).sum()
    }

    pub fn total_memory_bytes(&self) -> u64 {
        self.samples.iter().map(|s| s.memory_bytes).sum()
    }
}

/// A gas profiler that records call stacks and resource usage during a test.
///
/// # Example
///
/// ```ignore
/// use crucible::prelude::*;
/// use crucible::profiler::GasProfiler;
///
/// let env = MockEnv::builder().build();
/// let mut profiler = GasProfiler::new();
///
/// let profile = profiler.profile(|| {
///     // contract call here
/// });
///
/// println!("Total cost: {}", profile.total_cost());
/// ```
#[derive(Debug, Clone, Default)]
pub struct GasProfiler {
    current_stack: Vec<Frame>,
    profiles: Vec<Profile>,
    active: bool,
}

impl GasProfiler {
    /// Creates a new, inactive profiler.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` if the profiler is currently recording.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Starts recording a new profile.
    pub fn start(&mut self) {
        self.current_stack.clear();
        self.active = true;
    }

    /// Stops recording and returns the completed [`Profile`].
    pub fn stop(&mut self) -> Profile {
        self.active = false;
        let mut profile = Profile::new();
        profile.samples = std::mem::take(&mut self.profiles.last_mut().unwrap_or(&mut Profile::new()).samples);
        profile.finish();
        profile
    }

    /// Pushes a new frame onto the current call stack.
    pub fn enter(&mut self, frame: Frame) {
        if self.active {
            self.current_stack.push(frame);
        }
    }

    /// Pops the most recent frame and records a sample.
    pub fn exit(&mut self, cost: u64, memory_bytes: u64) {
        if self.active && !self.current_stack.is_empty() {
            let stack = std::mem::take(&mut self.current_stack);
            let sample = Sample::new(stack, cost, memory_bytes);
            if let Some(last) = self.profiles.last_mut() {
                last.add_sample(sample);
            } else {
                let mut profile = Profile::new();
                profile.add_sample(sample);
                self.profiles.push(profile);
            }
        }
    }

    /// Profiles a closure, returning the [`Profile`] and the closure's return value.
    pub fn profile<F, T>(&mut self, f: F) -> (Profile, T)
    where
        F: FnOnce() -> T,
    {
        self.start();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
        let profile = self.stop();
        let value = match result {
            Ok(v) => v,
            Err(payload) => {
                std::panic::resume_unwind(payload);
            }
        };
        (profile, value)
    }

    /// Returns all recorded profiles.
    pub fn profiles(&self) -> &[Profile] {
        &self.profiles
    }

    /// Clears all recorded profiles.
    pub fn clear(&mut self) {
        self.profiles.clear();
        self.current_stack.clear();
    }
}

/// Exports a [`Profile`] to speedscope-compatible JSON.
///
/// The returned string can be written to a `.json` file and opened in
/// [speedscope](https://www.speedscope.app/) for interactive exploration.
pub fn export_speedscope(profile: &Profile) -> String {
    let mut frames: Vec<(usize, Frame)> = Vec::new();
    let mut frame_index: HashMap<String, usize> = HashMap::new();
    let mut stacks: Vec<Vec<usize>> = Vec::new();
    let mut weights: Vec<String> = Vec::new();

    for sample in &profile.samples {
        let mut stack_indices: Vec<usize> = Vec::new();
        for frame in &sample.stack {
            let key = format!("{}/{}", frame.function, frame.name);
            let idx = *frame_index.entry(key.clone()).or_insert_with(|| {
                let idx = frames.len();
                frames.push((idx, frame.clone()));
                idx
            });
            stack_indices.push(idx);
        }
        stacks.push(stack_indices);
        weights.push(sample.cost.to_string());
    }

    let mut json = String::from("{\n");
    json.push_str("  \"name\": \"Crucible Profile\",\n");
    json.push_str("  \"profiles\": [\n");
    json.push_str("    {\n");
    json.push_str("      \"name\": \"Crucible Gas Profile\",\n");
    json.push_str("      \"unit\": \"instructions\",\n");
    json.push_str("      \"startValue\": 0,\n");
    json.push_str(&format!("      \"endValue\": {},\n", profile.total_cost()));
    json.push_str("      \"frames\": [\n");

    for (i, (_, frame)) in frames.iter().enumerate() {
        let comma = if i + 1 == frames.len() { "" } else { "," };
        json.push_str(&format!(
            "        {{\"name\": \"{}\", \"file\": \"{}\", \"line\": {}}}{}\n",
            escape_json(&frame.name),
            escape_json(frame.file.as_deref().unwrap_or("")),
            frame.line.unwrap_or(0),
            comma
        ));
    }

    json.push_str("      ],\n");
    json.push_str("      \"events\": [\n");

    for (i, (stack, weight)) in stacks.iter().zip(weights.iter()).enumerate() {
        let comma = if i + 1 == stacks.len() { "" } else { "," };
        let stack_json = stack
            .iter()
            .map(|idx| idx.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        json.push_str(&format!(
            "        {{\"type\": \"Sample\", \"stack\": [{}], \"weight\": {}}}{}\n",
            stack_json, weight, comma
        ));
    }

    json.push_str("      ]\n");
    json.push_str("    }\n");
    json.push_str("  ],\n");
    json.push_str("  \"shared\": {\n");
    json.push_str("    \"frames\": []\n");
    json.push_str("  }\n");
    json.push_str("}\n");
    json
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Exports a [`Profile`] to an SVG flamegraph.
///
/// Each rectangle represents a stack frame; width is proportional to the
/// frame's total cost across all samples.
pub fn export_flamegraph_svg(profile: &Profile) -> String {
    let width: u32 = 1200;
    let row_height: u32 = 18;
    let mut svg = String::new();

    let height = profile.samples.len().max(1) as u32 * row_height + 20;
    let bg = "#1e1e1e";
    svg.push_str(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}">
  <style>
    text {{ font-family: monospace; font-size: 12px; fill: #fff; }}
    rect {{ stroke: #000; stroke-width: 0.5; }}
  </style>
  <rect width="100%" height="100%" fill="{bg}"/>
"#,
        width, height, width, height, bg = bg
    ));

    let total_cost = profile.total_cost().max(1);

    for (sample_idx, sample) in profile.samples.iter().enumerate() {
        let y = sample_idx as u32 * row_height + 10;
        let mut x: f64 = 0.0;
        let mut remaining_width = width as f64;

        for (frame_idx, frame) in sample.stack.iter().enumerate() {
            let frame_cost = sample.cost.max(1) as f64 / sample.stack.len() as f64;
            let frame_width = (frame_cost / total_cost as f64) * remaining_width;
            let color = hue_for_depth(frame_idx);

            svg.push_str(&format!(
                r#"  <rect x="{x:.1}" y="{y}" width="{w:.1}" height="{h}" fill="{color}"/>
  <title>{func}::{name}</title>
"#,
                x = x,
                y = y,
                w = frame_width,
                h = row_height,
                color = color,
                func = frame.function,
                name = frame.name,
            ));

            if frame_width > 40.0 {
                svg.push_str(&format!(
                    r#"  <text x="{x:.1}" y="{ty:.1}">{label}</text>
"#,
                    x = x + 4.0,
                    ty = y as f64 + row_height as f64 - 4.0,
                    label = truncate(&frame.name, (frame_width / 7.0) as usize),
                ));
            }

            x += frame_width;
            remaining_width -= frame_width;
        }
    }

    svg.push_str("</svg>");
    svg
}

fn hue_for_depth(depth: usize) -> &'static str {
    match depth % 5 {
        0 => "#e6194b",
        1 => "#3cb44b",
        2 => "#ffe119",
        3 => "#4363d8",
        4 => "#f58231",
        _ => "#911eb4",
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        s.chars().take(max_len.saturating_sub(1)).collect::<String>() + "…"
    }
}

fn timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_records_samples() {
        let mut profiler = GasProfiler::new();
        profiler.start();
        profiler.enter(Frame::new("test_fn", "test_mod"));
        profiler.exit(100, 1024);
        let profile = profiler.stop();

        assert_eq!(profile.samples.len(), 1);
        assert_eq!(profile.total_cost(), 100);
        assert_eq!(profile.total_memory_bytes(), 1024);
    }

    #[test]
    fn test_export_speedscope_produces_valid_json() {
        let mut profile = Profile::new();
        profile.add_sample(Sample::new(
            vec![Frame::new("foo", "bar")],
            50,
            512,
        ));
        let json = export_speedscope(&profile);
        assert!(json.contains("Crucible Gas Profile"));
        assert!(json.contains("foo"));
    }

    #[test]
    fn test_export_flamegraph_svg_contains_rectangles() {
        let mut profile = Profile::new();
        profile.add_sample(Sample::new(
            vec![Frame::new("foo", "bar")],
            50,
            512,
        ));
        let svg = export_flamegraph_svg(&profile);
        assert!(svg.contains("<svg"));
        assert!(svg.contains("<rect"));
        assert!(svg.contains("bar"));
    }
}
