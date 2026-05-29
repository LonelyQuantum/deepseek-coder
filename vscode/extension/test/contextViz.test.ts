import assert from "node:assert/strict";
import test from "node:test";

import {
  CONTEXT_SOURCE_PREVIEW_LIMIT,
  contextVizFromEvent,
  emptyContextViz,
} from "../src/contextViz.js";
import type { AgentEventEnvelope } from "../src/rpcServer.js";

test("context viz builds token segments and source summaries from context events", () => {
  const snapshot = contextVizFromEvent(
    agentEvent({
      inputTokens: 1000,
      maxInputTokens: 4000,
      stablePrefixHash: "sha256:abc",
      stablePrefixTokens: 300,
      stablePrefixBudgetTokens: 1200,
      stablePrefixBudgetRatioPpm: 300000,
      dynamicPreludeTokens: 500,
      turnSuffixTokens: 200,
      cacheHitTokens: 128,
      cacheMissTokens: 64,
      estimator: {
        name: "utf8_bytes",
        exact: false,
        description: "UTF-8 byte estimate",
        calibration: {
          sampleCount: 3,
          inputUnit: "bytes",
          slopePpm: 900000,
          interceptTokens: -2,
          meanAbsolutePercentageErrorPpm: 120000,
        },
      },
      includedSources: [
        {
          kind: "workspace_manifest",
          required: false,
          title: "Workspace manifest",
          tokens: 300,
          reason: "stable project map",
        },
        {
          kind: "user_task",
          required: true,
          title: "User task",
          tokens: 200,
          reason: "current turn",
        },
      ],
      omittedSources: [
        {
          kind: "file",
          required: false,
          path: "large.log",
          estimatedTokens: 900,
          inclusionReason: "large optional file",
          omissionReason: "token_budget_exceeded",
        },
      ],
      sections: [
        { placement: "stable_prefix", tokens: 300, itemCount: 1 },
        { placement: "dynamic_prelude", tokens: 500, itemCount: 2 },
        { placement: "turn_suffix", tokens: 200, itemCount: 1 },
      ],
      manifest: {
        manifestHash: "sha256:manifest",
        maxEntries: 500,
        totalDiscoveredFiles: 640,
        includedFiles: 500,
        omitted: [{ reason: "max_entries_exceeded", count: 140 }],
      },
    }),
  );

  assert.ok(snapshot);
  assert.equal(snapshot.status, "ready");
  assert.equal(snapshot.inputPercent, 25);
  assert.equal(snapshot.stablePrefixBudgetPercent, 25);
  assert.equal(snapshot.stablePrefixBudgetRatioPercent, 30);
  assert.deepEqual(
    snapshot.segments.map((segment) => ({
      placement: segment.placement,
      tokens: segment.tokens,
      percent: segment.percent,
      itemCount: segment.itemCount,
    })),
    [
      { placement: "stable_prefix", tokens: 300, percent: 30, itemCount: 1 },
      { placement: "dynamic_prelude", tokens: 500, percent: 50, itemCount: 2 },
      { placement: "turn_suffix", tokens: 200, percent: 20, itemCount: 1 },
    ],
  );
  assert.equal(snapshot.includedSourceCount, 2);
  assert.equal(snapshot.includedSources[0]?.label, "Workspace manifest");
  assert.equal(snapshot.omittedSourceCount, 1);
  assert.equal(snapshot.omittedSources[0]?.label, "large.log");
  assert.equal(snapshot.omittedSources[0]?.omissionReason, "token_budget_exceeded");
  assert.equal(snapshot.estimator?.calibration?.interceptTokens, -2);
  assert.equal(snapshot.manifest?.omitted[0]?.count, 140);
});

test("context viz keeps only the largest sources in the preview", () => {
  const includedSources = Array.from({ length: CONTEXT_SOURCE_PREVIEW_LIMIT + 2 }, (_, index) => ({
    kind: "file",
    required: false,
    path: `src/file_${index}.rs`,
    tokens: index + 1,
    reason: "ranked by size",
  }));

  const snapshot = contextVizFromEvent(
    agentEvent({
      inputTokens: 100,
      maxInputTokens: 1000,
      stablePrefixHash: "sha256:abc",
      stablePrefixTokens: 20,
      stablePrefixBudgetTokens: 300,
      stablePrefixBudgetRatioPpm: 300000,
      dynamicPreludeTokens: 30,
      turnSuffixTokens: 50,
      includedSources,
      omittedSources: [],
      sections: [],
    }),
  );

  assert.ok(snapshot);
  assert.equal(snapshot.includedSourceCount, CONTEXT_SOURCE_PREVIEW_LIMIT + 2);
  assert.equal(snapshot.includedSources.length, CONTEXT_SOURCE_PREVIEW_LIMIT);
  assert.equal(snapshot.includedSources[0]?.label, "src/file_9.rs");
});

test("context viz ignores non-context events and malformed context payloads", () => {
  assert.equal(contextVizFromEvent(agentEvent({}, "run.started")), undefined);
  assert.equal(contextVizFromEvent(agentEvent({ inputTokens: 10 })), undefined);
  assert.deepEqual(emptyContextViz(), {
    status: "empty",
    inputTokens: 0,
    maxInputTokens: 0,
    inputPercent: 0,
    stablePrefixBudgetTokens: 0,
    stablePrefixBudgetPercent: 0,
    stablePrefixBudgetRatioPercent: 0,
    segments: [],
    includedSourceCount: 0,
    omittedSourceCount: 0,
    includedSources: [],
    omittedSources: [],
  });
});

function agentEvent(payload: unknown, type = "context.built"): AgentEventEnvelope {
  return {
    seq: 7,
    time: "1970-01-01T00:00:00.000Z",
    type,
    runId: "run_1",
    turnId: "turn_1",
    payload,
  };
}
