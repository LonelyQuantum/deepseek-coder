export const OPEN_CHAT_COMMAND = "deepseek-coder.openChat";
export const OPEN_CHAT_NO_WORKSPACE_MESSAGE =
  "Open a trusted workspace before starting the deepseek-coder RPC server.";
export const APPROVAL_APPROVE_LABEL = "Approve";
export const APPROVAL_APPROVE_ONCE_LABEL = "Approve Once";
export const APPROVAL_APPROVE_SESSION_LABEL = "Approve For Session";
export const APPROVAL_REJECT_LABEL = "Reject";
export const APPROVAL_DISMISSED_REASON = "approval prompt dismissed";
export const APPROVAL_REJECTED_REASON = "rejected in VS Code";

export interface CommandRegistry {
  registerCommand(command: string, callback: () => unknown): DisposableLike;
}

export interface WindowMessenger {
  showInformationMessage(message: string): unknown;
  showWarningMessage?(message: string): unknown;
}

export interface RpcServerStarter {
  readonly status: string;
  start(): Promise<{
    readonly server: {
      readonly name: string;
      readonly version: string;
    };
  }>;
}

export interface ApprovalWindowMessenger {
  showWarningMessage(
    message: string,
    options: { modal: true },
    ...items: string[]
  ): string | undefined | PromiseLike<string | undefined>;
}

export interface DisposableLike {
  dispose(): unknown;
}

export type ApprovalPersistence = "never" | "session" | "workspace";

export interface ApprovalPromptRequest {
  readonly approvalId: string;
  readonly toolCallId: string;
  readonly toolName: string;
  readonly risk: string;
  readonly title: string;
  readonly detail: string;
  readonly persistable: boolean;
  readonly command?: string;
  readonly paths?: readonly string[];
}

export type ApprovalPromptDecision =
  | {
      readonly kind: "approve";
      readonly approvalId: string;
      readonly persist: ApprovalPersistence;
    }
  | {
      readonly kind: "reject";
      readonly approvalId: string;
      readonly reason: string;
    };

export function registerOpenChatCommand(
  commands: CommandRegistry,
  window: WindowMessenger,
  rpcServer?: RpcServerStarter,
): DisposableLike {
  return commands.registerCommand(OPEN_CHAT_COMMAND, () => {
    if (rpcServer === undefined) {
      return window.showInformationMessage(OPEN_CHAT_NO_WORKSPACE_MESSAGE);
    }

    return rpcServer
      .start()
      .then((ready) =>
        window.showInformationMessage(
          `deepseek-coder RPC server ready: ${ready.server.name} ${ready.server.version}`,
        ),
      )
      .catch((error: unknown) => {
        const message = `deepseek-coder RPC server failed to start: ${errorMessage(error)}`;
        if (window.showWarningMessage !== undefined) {
          return window.showWarningMessage(message);
        }

        return window.showInformationMessage(message);
      });
  });
}

export async function requestApproval(
  window: ApprovalWindowMessenger,
  request: ApprovalPromptRequest,
): Promise<ApprovalPromptDecision> {
  const choices = request.persistable
    ? [APPROVAL_APPROVE_ONCE_LABEL, APPROVAL_APPROVE_SESSION_LABEL, APPROVAL_REJECT_LABEL]
    : [APPROVAL_APPROVE_LABEL, APPROVAL_REJECT_LABEL];
  const selected = await window.showWarningMessage(
    formatApprovalMessage(request),
    { modal: true },
    ...choices,
  );

  if (selected === APPROVAL_APPROVE_LABEL || selected === APPROVAL_APPROVE_ONCE_LABEL) {
    return {
      kind: "approve",
      approvalId: request.approvalId,
      persist: "never",
    };
  }

  if (selected === APPROVAL_APPROVE_SESSION_LABEL) {
    return {
      kind: "approve",
      approvalId: request.approvalId,
      persist: "session",
    };
  }

  if (selected === APPROVAL_REJECT_LABEL) {
    return {
      kind: "reject",
      approvalId: request.approvalId,
      reason: APPROVAL_REJECTED_REASON,
    };
  }

  return {
    kind: "reject",
    approvalId: request.approvalId,
    reason: APPROVAL_DISMISSED_REASON,
  };
}

function formatApprovalMessage(request: ApprovalPromptRequest): string {
  const detail = [
    request.title,
    request.detail,
    `Tool: ${request.toolName}`,
    `Risk: ${request.risk}`,
  ];

  if (request.command !== undefined) {
    detail.push(`Command: ${request.command}`);
  }

  if (request.paths !== undefined && request.paths.length > 0) {
    detail.push(`Paths: ${request.paths.join(", ")}`);
  }

  return detail.join("\n");
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
