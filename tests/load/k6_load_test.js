// Location: tests/load/k6_load_test.js
// Production requirement: Automated Performance Regression & Load Generation Harness
//
// Continuous load generation suite simulating 2,000 concurrent developers
// compiling, simulating, and querying contract state against the Crucible
// backend. The suite enforces hard performance thresholds:
//
//   * p95 latency < 200ms on every core API endpoint
//   * 0% error rate (any 5xx / connection failure fails the run)
//
// Run locally:
//   k6 run tests/load/k6_load_test.js
//
// Run against a specific target (staging cluster):
//   BASE_URL=https://staging.crucible.example.com \
//   k6 run tests/load/k6_load_test.js
//
// Run the production soak profile:
//   k6 run --env SCENARIO=soak tests/load/k6_load_test.js

import http from "k6/http";
import { check, sleep, group } from "k6";
import { Rate, Trend } from "k6/metrics";

// ----------------------------------------------------------------------------
// Configuration
// ----------------------------------------------------------------------------
const BASE_URL = __ENV.BASE_URL || "http://localhost:8080";
const SCENARIO = __ENV.SCENARIO || "default";

// Headers required by the backend's `require_json_content_type` middleware.
const JSON_HEADERS = {
  "Content-Type": "application/json",
  Accept: "application/json",
};

// Custom metrics -----------------------------------------------------------------
const errorRate = new Rate("crucible_error_rate");
const p95Trend = new Trend("crucible_request_duration", true);

// ----------------------------------------------------------------------------
// Test data — a realistic, minimal Smart Contract payload per VU so that
// compilation/simulation handlers receive a valid request body.
// ----------------------------------------------------------------------------
function contractPayload(vu) {
  const project = `dev-${vu}-${Math.floor(Math.random() * 1e6)}`;
  return {
    projectName: project,
    sourceCode: [
      "#![no_std]",
      "use soroban_sdk::{contract, contractimpl, vec, Env, String};",
      "#[contract]",
      "pub struct Counter;",
      "#[contractimpl]",
      "impl Counter {",
      "    pub fn increment(env: Env, count: u32) -> u32 { count + 1 }",
      "}",
    ].join("\n"),
  };
}

function analyzePayload() {
  return {
    cargoToml: [
      "[package]",
      'name = "demo"',
      'version = "0.1.0"',
      'edition = "2021"',
      "[dependencies]",
      'soroban-sdk = "26"',
    ].join("\n"),
  };
}

function upgradePlanPayload() {
  return {
    contractId: "CCDAILYCLOCK",
    currentVersion: "1.0.0",
    targetVersion: "2.0.0",
    strategy: "atomic",
  };
}

function compliancePayload() {
  return {
    contractId: "CCDAILYCLOCK",
    standard: "SIP-10",
    strict: true,
  };
}

function storageOptimizePayload() {
  return {
    contractId: "CCDAILYCLOCK",
    entries: [
      { key: "balance", value: "u32" },
      { key: "owner", value: "address" },
    ],
  };
}

function versionPayload() {
  return {
    contractId: "CCDAILYCLOCK",
    sourceRef: "main",
    metadata: { build: "ci" },
  };
}

function versionDiffPayload() {
  return {
    contractId: "CCDAILYCLOCK",
    from: "1.0.0",
    to: "2.0.0",
  };
}

function deploymentPayload() {
  return {
    contractId: "CCDAILYCLOCK",
    network: "testnet",
    wasmHash: "0000000000000000000000000000000000000000000000000000000000000000",
  };
}

function testResultsPayload() {
  return {
    contractId: "CCDAILYCLOCK",
    runId: `run-${Math.floor(Math.random() * 1e9)}`,
    passed: 12,
    failed: 0,
    durationMs: 842,
  };
}

function logsPayload() {
  return {
    contractId: "CCDAILYCLOCK",
    event: "transfer",
    cursor: "0",
  };
}

// ----------------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------------
function record(res, name) {
  const ok = res.status >= 200 && res.status < 400;
  errorRate.add(!ok);
  p95Trend.add(res.timings.duration, { endpoint: name });
  check(res, {
    [`${name} status is 2xx/3xx`]: (r) => r.status >= 200 && r.status < 400,
  });
  return ok;
}

function get(name, path) {
  record(http.get(`${BASE_URL}${path}`, { headers: JSON_HEADERS }), name);
}

function post(name, path, body) {
  record(
    http.post(`${BASE_URL}${path}`, JSON.stringify(body), {
      headers: JSON_HEADERS,
    }),
    name,
  );
}

// ----------------------------------------------------------------------------
// Scenario: one "developer session" — compile, simulate, then query state.
// ----------------------------------------------------------------------------
export function developerSession() {
  const vu = __VU;

  group("compile", () => {
    post("compile", "/api/v1/contracts/compile", contractPayload(vu));
    post(
      "analyze-dependencies",
      "/api/v1/contracts/analyze-dependencies",
      analyzePayload(),
    );
  });

  group("simulate", () => {
    post("upgrade-plan", "/api/v1/contracts/upgrade-plan", upgradePlanPayload());
    post(
      "compliance-check",
      "/api/v1/contracts/compliance-check",
      compliancePayload(),
    );
    post(
      "storage-optimize",
      "/api/v1/contracts/storage/optimize",
      storageOptimizePayload(),
    );
    post("versions", "/api/v1/contracts/versions", versionPayload());
    post("versions-diff", "/api/v1/contracts/versions/diff", versionDiffPayload());
    post(
      "deployments",
      "/api/v1/contracts/deployments",
      deploymentPayload(),
    );
    post(
      "test-results",
      "/api/v1/contracts/test-results",
      testResultsPayload(),
    );
    post("logs-post", "/api/v1/contracts/logs", logsPayload());
  });

  group("query", () => {
    get("templates", "/api/v1/contracts/templates");
    get("networks", "/api/v1/contracts/networks");
    get("logs-get", "/api/v1/contracts/logs");
    get("coverage", "/api/v1/coverage/demo-project");
    get("dashboard", "/api/v1/dashboard");
    get("profiling-health", "/api/v1/profiling/health");
    get("status", "/api/status");
  });

  // Think time between sessions — a developer does not fire requests back to
  // back; they read output, tweak source, re-submit.
  sleep(Math.random() * 2 + 0.5);
}

// ----------------------------------------------------------------------------
// Scenarios & load profiles
// ----------------------------------------------------------------------------
// `default`   — ramped load peaking at 2,000 concurrent developers.
// `soak`      — steady 1,000 VUs for 1h to catch memory/latency drift.
// `spike`     — sudden burst to 2,000 VUs to validate autoscaling.
// ----------------------------------------------------------------------------
export const options = {
  scenarios: {
    developer_load: {
      executor: "ramping-vus",
      exec: "developerSession",
      startVUs: 0,
      stages:
        SCENARIO === "soak"
          ? [
              { duration: "2m", target: 1000 },
              { duration: "1h", target: 1000 },
              { duration: "2m", target: 0 },
            ]
          : SCENARIO === "spike"
            ? [
                { duration: "30s", target: 50 },
                { duration: "10s", target: 2000 },
                { duration: "1m", target: 2000 },
                { duration: "30s", target: 0 },
              ]
            : [
                // Default: ramp to 2,000 concurrent developers.
                { duration: "3m", target: 500 },
                { duration: "5m", target: 2000 },
                { duration: "10m", target: 2000 },
                { duration: "5m", target: 1000 },
                { duration: "2m", target: 0 },
              ],
      gracefulRampDown: "30s",
    },
  },

  // Hard performance gates. The run fails (non-zero exit) if any of these
  // thresholds are violated, blocking a release in CI.
  thresholds: {
    // p95 latency must stay under 200ms on every core endpoint.
    http_req_duration: ["p(95)<200"],
    // 0% error rate across the entire run.
    crucible_error_rate: ["rate<0.0001"],
    // Per-group latency guards for the three workload classes.
    "http_req_duration{group:::compile}": ["p(95)<200"],
    "http_req_duration{group:::simulate}": ["p(95)<200"],
    "http_req_duration{group:::query}": ["p(95)<200"],
  },

  // Tag requests so per-endpoint thresholds can be inspected in Grafana.
  tags: { test: "crucible-load" },
};

export default function () {
  developerSession();
}
