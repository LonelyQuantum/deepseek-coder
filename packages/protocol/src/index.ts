export const protocolVersion = "0.1.0" as const;

export const riskLevels = ["read", "write", "exec", "network", "destructive"] as const;
export type RiskLevel = (typeof riskLevels)[number];
export type ApprovalRisk = RiskLevel;

export const approvalRequirements = ["none", "required", "always_required"] as const;
export type ApprovalRequirement = (typeof approvalRequirements)[number];

export const riskDefaultApproval = {
  read: "none",
  write: "required",
  exec: "required",
  network: "required",
  destructive: "always_required",
} as const satisfies Record<RiskLevel, ApprovalRequirement>;

export const approvalPersistences = ["never", "session", "workspace"] as const;
export type ApprovalPersistence = (typeof approvalPersistences)[number];

export const approvalStates = [
  "pending",
  "approved",
  "executing",
  "completed",
  "failed",
  "rejected",
  "canceled",
  "expired",
] as const;
export type ApprovalState = (typeof approvalStates)[number];

export const approvalStateTransitions: Record<ApprovalState, readonly ApprovalState[]> = {
  pending: ["approved", "rejected", "canceled", "expired"],
  approved: ["executing"],
  executing: ["completed", "failed"],
  completed: [],
  failed: [],
  rejected: [],
  canceled: [],
  expired: [],
};

export function canTransitionApprovalState(from: ApprovalState, to: ApprovalState): boolean {
  return approvalStateTransitions[from].includes(to);
}

export function isApprovalRequired(risk: RiskLevel): boolean {
  return riskDefaultApproval[risk] !== "none";
}

export type JsonSchema = Readonly<Record<string, unknown>>;

export const toolImplementationStatuses = ["schema_only", "executor_implemented"] as const;
export type ToolImplementationStatus = (typeof toolImplementationStatuses)[number];

export const toolNames = [
  "workspace_manifest",
  "read_file",
  "search",
  "apply_patch",
  "shell",
  "git_status",
  "git_diff",
  "lsp_diagnostics",
  "plan_update",
] as const;
export type ToolName = (typeof toolNames)[number];

export interface ToolDefinition {
  readonly name: ToolName;
  readonly description: string;
  readonly risk: RiskLevel;
  readonly approval: ApprovalRequirement;
  readonly implementationStatus: ToolImplementationStatus;
  readonly argumentSchema: JsonSchema;
  readonly resultSchema: JsonSchema;
}

const statusResultSchema = {
  type: "object",
  additionalProperties: false,
  required: ["status", "summary"],
  properties: {
    status: { type: "string", enum: ["ok", "failed"] },
    summary: { type: "string" },
    errorCode: { type: "string" },
  },
} as const satisfies JsonSchema;

export const toolDefinitions = [
  {
    name: "workspace_manifest",
    description: "生成 workspace manifest。",
    risk: "read",
    approval: "none",
    implementationStatus: "schema_only",
    argumentSchema: {
      type: "object",
      additionalProperties: false,
      properties: {
        root: { type: "string" },
        respectGitignore: { type: "boolean" },
      },
    },
    resultSchema: statusResultSchema,
  },
  {
    name: "read_file",
    description: "读取 workspace 内 UTF-8 文本文件。",
    risk: "read",
    approval: "none",
    implementationStatus: "executor_implemented",
    argumentSchema: {
      type: "object",
      additionalProperties: false,
      required: ["path"],
      properties: {
        path: { type: "string", minLength: 1 },
        startLine: { type: "integer", minimum: 1 },
        endLine: { type: "integer", minimum: 1 },
      },
    },
    resultSchema: statusResultSchema,
  },
  {
    name: "search",
    description: "使用 ripgrep 搜索 workspace 文本。",
    risk: "read",
    approval: "none",
    implementationStatus: "executor_implemented",
    argumentSchema: {
      type: "object",
      additionalProperties: false,
      required: ["query"],
      properties: {
        query: { type: "string", minLength: 1 },
        paths: { type: "array", items: { type: "string" } },
        caseSensitive: { type: "boolean" },
        maxResults: { type: "integer", minimum: 1 },
      },
    },
    resultSchema: statusResultSchema,
  },
  {
    name: "apply_patch",
    description: "应用统一 diff patch。",
    risk: "write",
    approval: "required",
    implementationStatus: "executor_implemented",
    argumentSchema: {
      type: "object",
      additionalProperties: false,
      required: ["unifiedDiff", "expectedFiles"],
      properties: {
        unifiedDiff: { type: "string", minLength: 1 },
        expectedFiles: {
          type: "array",
          minItems: 1,
          items: { type: "string", minLength: 1 },
        },
      },
    },
    resultSchema: statusResultSchema,
  },
  {
    name: "shell",
    description: "执行非交互式 shell 命令。",
    risk: "exec",
    approval: "required",
    implementationStatus: "executor_implemented",
    argumentSchema: {
      type: "object",
      additionalProperties: false,
      required: ["command"],
      properties: {
        command: { type: "string", minLength: 1 },
        cwd: { type: "string" },
        timeoutMs: { type: "integer", minimum: 1 },
      },
    },
    resultSchema: statusResultSchema,
  },
  {
    name: "git_status",
    description: "读取 git status。",
    risk: "read",
    approval: "none",
    implementationStatus: "executor_implemented",
    argumentSchema: {
      type: "object",
      additionalProperties: false,
      properties: {
        porcelain: { type: "boolean" },
      },
    },
    resultSchema: statusResultSchema,
  },
  {
    name: "git_diff",
    description: "读取 git diff。",
    risk: "read",
    approval: "none",
    implementationStatus: "executor_implemented",
    argumentSchema: {
      type: "object",
      additionalProperties: false,
      properties: {
        staged: { type: "boolean" },
        paths: { type: "array", items: { type: "string" } },
      },
    },
    resultSchema: statusResultSchema,
  },
  {
    name: "lsp_diagnostics",
    description: "读取语言服务器或编辑器 diagnostics。",
    risk: "read",
    approval: "none",
    implementationStatus: "schema_only",
    argumentSchema: {
      type: "object",
      additionalProperties: false,
      properties: {
        paths: { type: "array", items: { type: "string" } },
      },
    },
    resultSchema: statusResultSchema,
  },
  {
    name: "plan_update",
    description: "更新当前计划。",
    risk: "read",
    approval: "none",
    implementationStatus: "schema_only",
    argumentSchema: {
      type: "object",
      additionalProperties: false,
      required: ["steps"],
      properties: {
        steps: {
          type: "array",
          items: {
            type: "object",
            additionalProperties: false,
            required: ["id", "title", "status"],
            properties: {
              id: { type: "string" },
              title: { type: "string" },
              status: {
                type: "string",
                enum: ["pending", "in_progress", "completed", "failed", "canceled"],
              },
              detail: { type: "string" },
            },
          },
        },
      },
    },
    resultSchema: statusResultSchema,
  },
] as const satisfies readonly ToolDefinition[];

export function findToolDefinition(name: string): ToolDefinition | undefined {
  return toolDefinitions.find((tool) => tool.name === name);
}

export interface AgentInitializeParams {
  readonly workspacePath: string;
  readonly protocolVersion: typeof protocolVersion;
}

export interface ApprovalRequest {
  readonly id: string;
  readonly risk: RiskLevel;
  readonly title: string;
  readonly detail: string;
  readonly toolCallId?: string;
  readonly toolName?: ToolName;
  readonly command?: string;
  readonly paths?: readonly string[];
  readonly persistable: boolean;
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
