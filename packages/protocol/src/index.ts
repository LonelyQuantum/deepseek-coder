export const protocolVersion = "0.1.0" as const;

export type ApprovalRisk = "read" | "write" | "exec" | "network" | "destructive";

export interface AgentInitializeParams {
  readonly workspacePath: string;
  readonly protocolVersion: typeof protocolVersion;
}

export interface ApprovalRequest {
  readonly id: string;
  readonly risk: ApprovalRisk;
  readonly title: string;
  readonly detail: string;
}

export type AgentEvent =
  | {
      readonly type: "delta";
      readonly text: string;
    }
  | {
      readonly type: "approvalRequired";
      readonly request: ApprovalRequest;
    }
  | {
      readonly type: "done";
    };
