import assert from "node:assert/strict";
import test from "node:test";

import {
  APPROVAL_APPROVE_ONCE_LABEL,
  APPROVAL_APPROVE_SESSION_LABEL,
  APPROVAL_DISMISSED_REASON,
  APPROVAL_REJECTED_REASON,
  APPROVAL_REJECT_LABEL,
  OPEN_CHAT_COMMAND,
  OPEN_CHAT_NO_WORKSPACE_MESSAGE,
  type ApprovalPromptRequest,
  type ApprovalWindowMessenger,
  type CommandRegistry,
  type DisposableLike,
  type WindowMessenger,
  registerOpenChatCommand,
  requestApproval,
} from "../src/commands.js";

test("registerOpenChatCommand registers the public command id", () => {
  const disposable: DisposableLike = { dispose: () => undefined };
  let registeredCommand: string | undefined;

  const commands: CommandRegistry = {
    registerCommand(command) {
      registeredCommand = command;
      return disposable;
    },
  };

  const window: WindowMessenger = {
    showInformationMessage: () => undefined,
  };

  const returned = registerOpenChatCommand(commands, window);

  assert.equal(registeredCommand, OPEN_CHAT_COMMAND);
  assert.equal(returned, disposable);
});

test("open chat command asks for a workspace when RPC server is unavailable", () => {
  let callback: (() => unknown) | undefined;
  let message: string | undefined;

  const commands: CommandRegistry = {
    registerCommand(_command, registeredCallback) {
      callback = registeredCallback;
      return { dispose: () => undefined };
    },
  };

  const window: WindowMessenger = {
    showInformationMessage(value) {
      message = value;
    },
  };

  registerOpenChatCommand(commands, window);
  assert.ok(callback);

  callback();

  assert.equal(message, OPEN_CHAT_NO_WORKSPACE_MESSAGE);
  assert.ok(message?.includes("trusted workspace"));
});

test("open chat command starts the RPC server and reports readiness", async () => {
  let callback: (() => unknown) | undefined;
  let message: string | undefined;

  const commands: CommandRegistry = {
    registerCommand(_command, registeredCallback) {
      callback = registeredCallback;
      return { dispose: () => undefined };
    },
  };

  const window: WindowMessenger = {
    showInformationMessage(value) {
      message = value;
    },
  };

  registerOpenChatCommand(commands, window, {
    status: "stopped",
    async start() {
      return {
        server: {
          name: "prole-coder-agent-rpc",
          version: "0.1.0",
        },
      };
    },
  });
  assert.ok(callback);

  await callback();

  assert.ok(message?.includes("RPC server ready"));
  assert.ok(message?.includes("prole-coder-agent-rpc"));
});

test("requestApproval maps VS Code approve choices to approval params", async () => {
  const approvals = [APPROVAL_APPROVE_ONCE_LABEL, APPROVAL_APPROVE_SESSION_LABEL] as const;

  for (const selected of approvals) {
    let message: string | undefined;
    let modal: boolean | undefined;
    let items: readonly string[] = [];
    const window: ApprovalWindowMessenger = {
      showWarningMessage(value, options, ...choices) {
        message = value;
        modal = options.modal;
        items = choices;
        return selected;
      },
    };

    const decision = await requestApproval(window, sampleApprovalRequest(true));

    assert.equal(decision.kind, "approve");
    assert.equal(decision.approvalId, "approval_1");
    assert.equal(
      decision.persist,
      selected === APPROVAL_APPROVE_SESSION_LABEL ? "session" : "never",
    );
    assert.equal(modal, true);
    assert.ok(message?.includes("Command: cargo test"));
    assert.ok(items.includes(APPROVAL_REJECT_LABEL));
  }
});

test("requestApproval maps reject and dismiss to reject decisions", async () => {
  const rejectWindow: ApprovalWindowMessenger = {
    showWarningMessage() {
      return APPROVAL_REJECT_LABEL;
    },
  };
  const rejected = await requestApproval(rejectWindow, sampleApprovalRequest(false));

  assert.deepEqual(rejected, {
    kind: "reject",
    approvalId: "approval_1",
    reason: APPROVAL_REJECTED_REASON,
  });

  const dismissWindow: ApprovalWindowMessenger = {
    showWarningMessage() {
      return undefined;
    },
  };
  const dismissed = await requestApproval(dismissWindow, sampleApprovalRequest(false));

  assert.deepEqual(dismissed, {
    kind: "reject",
    approvalId: "approval_1",
    reason: APPROVAL_DISMISSED_REASON,
  });
});

function sampleApprovalRequest(persistable: boolean): ApprovalPromptRequest {
  return {
    approvalId: "approval_1",
    toolCallId: "tool_call_1",
    toolName: "shell",
    risk: "exec",
    title: "Execute shell command",
    detail: "Run verification",
    persistable,
    command: "cargo test",
    paths: ["crates/cli/src/lib.rs"],
  };
}
