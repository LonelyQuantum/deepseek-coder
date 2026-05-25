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
  type ApprovalRequest,
  type ApproveParams,
  type ApproveResult,
  type CancelParams,
  type CancelResult,
  type ListRunsParams,
  type ListRunsResult,
  type RunSummary,
  type RejectParams,
  type RejectResult,
  type ToolApprovalRequiredPayload,
  type ToolApprovalResolvedPayload,
  approvalStateTransitions,
  canTransitionApprovalState,
  jsonRpcVersion,
  findToolDefinition,
  isApprovalRequired,
  protocolErrorDefinitions,
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
const jsonRpcProtocolDocument = readFileSync(
  new URL("../../../../docs/json-rpc-protocol.md", import.meta.url),
  "utf8",
);

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

test("protocol error code registry matches protocol document", () => {
  const codes = new Set<number>();
  const names = new Set<string>();

  for (const definition of protocolErrorDefinitions) {
    const displayName = definition.name.startsWith("E_")
      ? `\`${definition.name}\``
      : definition.name;
    const rowPrefix = `| ${definition.code} | ${displayName} |`;

    assert.equal(codes.has(definition.code), false, `duplicate error code ${definition.code}`);
    assert.equal(names.has(definition.name), false, `duplicate error name ${definition.name}`);
    assert.equal(
      jsonRpcProtocolDocument.includes(rowPrefix),
      true,
      `missing protocol document row starting with ${rowPrefix}`,
    );
    codes.add(definition.code);
    names.add(definition.name);
  }
});

test("approval request and decision params use stable protocol fields", () => {
  const request = {
    approvalId: "approval_1",
    toolCallId: "call_1",
    toolName: "shell",
    risk: "exec",
    title: "Run shell command",
    detail: "Execute cargo test",
    command: "cargo test",
    persistable: false,
  } satisfies ApprovalRequest;
  const requiredPayload = {
    approvalId: request.approvalId,
    toolCallId: request.toolCallId,
    toolName: request.toolName,
    risk: request.risk,
    title: request.title,
    detail: request.detail,
    command: request.command,
    persistable: request.persistable,
  } satisfies ToolApprovalRequiredPayload;
  const resolvedPayload = {
    approvalId: request.approvalId,
    toolCallId: request.toolCallId,
    toolName: request.toolName,
    decision: "approved",
  } satisfies ToolApprovalResolvedPayload;
  const approve = {
    approvalId: request.approvalId,
    persist: "session",
  } satisfies ApproveParams;
  const approveResult = {
    approvalId: request.approvalId,
    state: "approved",
    persist: "session",
  } satisfies ApproveResult;
  const reject = {
    approvalId: request.approvalId,
    reason: "not now",
  } satisfies RejectParams;
  const rejectResult = {
    approvalId: request.approvalId,
    state: "rejected",
    reason: "not now",
  } satisfies RejectResult;
  const cancel = {
    runId: "run_1",
    reason: "user canceled",
  } satisfies CancelParams;
  const cancelResult = {
    runId: cancel.runId,
    state: "canceled",
    reason: cancel.reason,
  } satisfies CancelResult;
  const expiredPayload = {
    approvalId: request.approvalId,
    toolCallId: request.toolCallId,
    toolName: request.toolName,
    decision: "expired",
    reason: "approval timed out",
  } satisfies ToolApprovalResolvedPayload;

  assert.equal(approve.approvalId, "approval_1");
  assert.equal(requiredPayload.toolName, "shell");
  assert.equal(resolvedPayload.decision, "approved");
  assert.equal(approveResult.state, "approved");
  assert.equal(reject.reason, rejectResult.reason);
  assert.equal(cancelResult.state, "canceled");
  assert.equal(expiredPayload.decision, "expired");
});

test("run summary params and results use stable protocol fields", () => {
  const params = {
    limit: 20,
  } satisfies ListRunsParams;
  const summary = {
    runId: "run_1",
    title: "Fix README",
    status: "completed",
    startedAt: "1970-01-01T00:00:00.000Z",
    updatedAt: "1970-01-01T00:00:02.000Z",
    completedAt: "1970-01-01T00:00:02.000Z",
    lastSeq: 8,
    eventCount: 8,
    mode: "edit",
    summary: "Updated README.",
    changedFiles: ["README.md"],
    verificationStatus: "passed",
  } satisfies RunSummary;
  const result = {
    runs: [summary],
  } satisfies ListRunsResult;

  assert.equal(params.limit, 20);
  assert.equal(result.runs[0]?.runId, "run_1");
  assert.equal(result.runs[0]?.status, "completed");
  assert.equal(result.runs[0]?.lastSeq, 8);
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

test("read_file result schema exposes file summary metadata", () => {
  const readFile = findToolDefinition("read_file");

  assert.ok(readFile);
  assert.deepEqual(readFile.resultSchema.required, [
    "status",
    "summary",
    "path",
    "content",
    "lineCount",
    "sha256",
    "sizeBytes",
  ]);
  assert.equal(
    (readFile.resultSchema.properties as Record<string, { pattern?: string }>).sha256?.pattern,
    "^[0-9a-f]{64}$",
  );
  assert.equal(
    (readFile.resultSchema.properties as Record<string, { minimum?: number }>).sizeBytes?.minimum,
    0,
  );
});

test("workspace_manifest is executable and exposes manifest metadata schema", () => {
  const workspaceManifest = findToolDefinition("workspace_manifest");

  assert.ok(workspaceManifest);
  assert.equal(workspaceManifest.implementationStatus, "executor_implemented");
  assert.deepEqual(workspaceManifest.resultSchema.required, [
    "status",
    "summary",
    "manifestHash",
    "summaryMarkdown",
    "manifest",
  ]);
  assert.equal(
    (workspaceManifest.resultSchema.properties as Record<string, { pattern?: string }>)
      .manifestHash?.pattern,
    "^sha256:[0-9a-f]{64}$",
  );
});
