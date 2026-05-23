import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  agentEventMethod,
  agentInitializeMethod,
  agentSendTurnMethod,
  agentResumeMethod,
  agentApproveMethod,
  agentRejectMethod,
  agentCancelMethod,
  agentListRunsMethod,
  approvalStateTransitions,
  canTransitionApprovalState,
  jsonRpcVersion,
  findToolDefinition,
  isApprovalRequired,
  protocolVersion,
  riskDefaultApproval,
  riskLevels,
  toolDefinitions,
  toolNames,
} from "../src/index.js";

interface ToolRegistryFixture {
  readonly version: string;
  readonly riskLevels: readonly string[];
  readonly riskDefaultApproval: Readonly<Record<string, string>>;
  readonly tools: readonly ToolRegistryTool[];
}

interface ToolRegistryTool {
  readonly name: string;
  readonly risk: string;
  readonly approval: string;
  readonly status: string;
}

const toolRegistryFixture = JSON.parse(
  readFileSync(
    new URL("../../../../docs/protocol/tool-registry.v1.json", import.meta.url),
    "utf8",
  ),
) as ToolRegistryFixture;

test("risk defaults stay aligned with approval requirement helper", () => {
  assert.deepEqual(riskDefaultApproval, {
    read: "none",
    write: "required",
    exec: "required",
    network: "required",
    destructive: "always_required",
  });

  for (const risk of riskLevels) {
    assert.equal(isApprovalRequired(risk), riskDefaultApproval[risk] !== "none");
  }
});

test("approval state transitions allow only documented next states", () => {
  assert.equal(canTransitionApprovalState("pending", "approved"), true);
  assert.equal(canTransitionApprovalState("pending", "rejected"), true);
  assert.equal(canTransitionApprovalState("approved", "executing"), true);
  assert.equal(canTransitionApprovalState("executing", "completed"), true);
  assert.equal(canTransitionApprovalState("executing", "failed"), true);

  assert.equal(canTransitionApprovalState("completed", "executing"), false);
  assert.equal(canTransitionApprovalState("rejected", "approved"), false);

  for (const terminal of ["completed", "failed", "rejected", "canceled", "expired"] as const) {
    assert.deepEqual(approvalStateTransitions[terminal], []);
  }
});

test("JSON-RPC method constants match protocol document", () => {
  assert.equal(jsonRpcVersion, "2.0");
  assert.equal(agentInitializeMethod, "agent.initialize");
  assert.equal(agentSendTurnMethod, "agent.sendTurn");
  assert.equal(agentResumeMethod, "agent.resume");
  assert.equal(agentApproveMethod, "agent.approve");
  assert.equal(agentRejectMethod, "agent.reject");
  assert.equal(agentCancelMethod, "agent.cancel");
  assert.equal(agentListRunsMethod, "agent.listRuns");
  assert.equal(agentEventMethod, "agent.event");
});

test("tool registry contains every declared tool exactly once", () => {
  const registeredNames = toolDefinitions.map((tool) => tool.name);

  assert.deepEqual([...registeredNames].sort(), [...toolNames].sort());
  assert.equal(new Set(registeredNames).size, toolNames.length);

  for (const name of toolNames) {
    assert.equal(findToolDefinition(name)?.name, name);
  }
  assert.equal(findToolDefinition("missing"), undefined);
});

test("tool registry stays aligned with shared protocol fixture", () => {
  assert.equal(toolRegistryFixture.version, protocolVersion);
  assert.deepEqual(toolRegistryFixture.riskLevels, [...riskLevels]);
  assert.deepEqual(toolRegistryFixture.riskDefaultApproval, riskDefaultApproval);
  assert.deepEqual(
    sortedTools(toolRegistryFixture.tools),
    sortedTools(toolDefinitions.map((tool) => ({
      name: tool.name,
      risk: tool.risk,
      approval: tool.approval,
      status: tool.implementationStatus,
    }))),
  );
});

function sortedTools(tools: readonly ToolRegistryTool[]): ToolRegistryTool[] {
  const names = new Set<string>();

  for (const tool of tools) {
    assert.equal(names.has(tool.name), false, `tool ${tool.name} must be declared only once`);
    names.add(tool.name);
  }

  return [...tools].sort((left, right) => left.name.localeCompare(right.name));
}

test("tool approval defaults match risk defaults", () => {
  for (const tool of toolDefinitions) {
    assert.equal(
      tool.approval,
      riskDefaultApproval[tool.risk],
      `${tool.name} approval should match its static risk`,
    );
  }
});

test("mutating and executing tools require approval", () => {
  assert.equal(findToolDefinition("apply_patch")?.risk, "write");
  assert.equal(findToolDefinition("apply_patch")?.approval, "required");
  assert.equal(findToolDefinition("shell")?.risk, "exec");
  assert.equal(findToolDefinition("shell")?.approval, "required");
});

test("tool schemas are explicit object schemas", () => {
  for (const tool of toolDefinitions) {
    assert.equal(tool.argumentSchema.type, "object", `${tool.name} arguments must be an object`);
    assert.equal(tool.resultSchema.type, "object", `${tool.name} result must be an object`);
  }
});
