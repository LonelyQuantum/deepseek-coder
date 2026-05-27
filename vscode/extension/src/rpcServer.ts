import { spawn } from "node:child_process";

import type { AgentEventEnvelope as ProtocolAgentEventEnvelope } from "@prole-coder/protocol" with {
  "resolution-mode": "import",
};

export const RPC_PROTOCOL_VERSION = "0.1.0";
export const RPC_INITIALIZE_METHOD = "agent.initialize";
export const RPC_EVENT_METHOD = "agent.event";
export const DEFAULT_RPC_COMMAND = "prole";
export const DEFAULT_RPC_ARGS = ["rpc"] as const;

export type RpcServerStatus = "stopped" | "starting" | "ready" | "failed";

export interface RpcServerLaunchConfig {
  readonly command: string;
  readonly args: readonly string[];
  readonly autoStart: boolean;
}

export interface RpcServerWorkspace {
  readonly root: string;
  readonly trusted: boolean;
}

export interface RpcServerReadyState {
  readonly protocolVersion: string;
  readonly server: {
    readonly name: string;
    readonly version: string;
  };
  readonly stateDir: string;
}

export interface RpcServerNotifier {
  info(message: string): unknown;
  warn(message: string): unknown;
}

export interface RpcServerConfiguration {
  get<T>(section: string, defaultValue: T): T;
}

export interface RpcSpawnOptions {
  readonly cwd: string;
}

export interface RpcWritable {
  write(data: string): unknown;
  end?(): unknown;
}

export interface RpcReadable {
  on(event: "data", listener: (chunk: Buffer | string) => void): unknown;
}

export interface RpcChildProcess {
  readonly stdin: RpcWritable | null;
  readonly stdout: RpcReadable | null;
  readonly stderr: RpcReadable | null;
  readonly killed?: boolean;
  kill(signal?: NodeJS.Signals | number): boolean;
  on(event: "exit", listener: (code: number | null, signal: NodeJS.Signals | null) => void): unknown;
  on(event: "error", listener: (error: Error) => void): unknown;
}

export interface RpcProcessFactory {
  spawn(command: string, args: readonly string[], options: RpcSpawnOptions): RpcChildProcess;
}

export type AgentEventEnvelope = ProtocolAgentEventEnvelope;

export interface DisposableLike {
  dispose(): unknown;
}

export interface RpcServerManagerOptions {
  readonly launch: RpcServerLaunchConfig;
  readonly workspace: RpcServerWorkspace;
  readonly extensionVersion: string;
  readonly processFactory?: RpcProcessFactory;
  readonly notifier?: RpcServerNotifier;
}

interface JsonRpcErrorObject {
  readonly code: number;
  readonly message: string;
  readonly data?: unknown;
}

interface JsonRpcResponse {
  readonly jsonrpc: string;
  readonly id: string | number | null;
  readonly result?: unknown;
  readonly error?: JsonRpcErrorObject;
}

interface JsonRpcNotification {
  readonly jsonrpc: string;
  readonly method: string;
  readonly params?: unknown;
}

interface PendingRpcRequest<TResult> {
  resolve(value: TResult): void;
  reject(error: Error): void;
}

export class RpcRequestError extends Error {
  readonly code: number;
  readonly data: unknown;

  constructor(error: JsonRpcErrorObject) {
    super(error.message);
    this.name = "RpcRequestError";
    this.code = error.code;
    this.data = error.data;
  }
}

export const nodeRpcProcessFactory: RpcProcessFactory = {
  spawn(command, args, options) {
    return spawn(command, [...args], {
      cwd: options.cwd,
      stdio: "pipe",
      windowsHide: true,
    });
  },
};

export function readRpcServerLaunchConfig(config: RpcServerConfiguration): RpcServerLaunchConfig {
  const command = config.get("command", DEFAULT_RPC_COMMAND).trim();
  if (command.length === 0) {
    throw new Error("prole-coder.rpc.command must not be empty");
  }

  const args = config.get<readonly string[]>("args", DEFAULT_RPC_ARGS);
  for (const arg of args) {
    if (typeof arg !== "string") {
      throw new Error("prole-coder.rpc.args must contain only strings");
    }
  }

  return {
    command,
    args: [...args],
    autoStart: config.get("autoStart", true),
  };
}

export class RpcServerManager implements DisposableLike {
  private readonly launch: RpcServerLaunchConfig;
  private readonly workspace: RpcServerWorkspace;
  private readonly extensionVersion: string;
  private readonly processFactory: RpcProcessFactory;
  private readonly notifier: RpcServerNotifier | undefined;
  private readonly eventHandlers = new Set<(event: AgentEventEnvelope) => void>();
  private readonly pendingRequests = new Map<string, PendingRpcRequest<unknown>>();

  private child: RpcChildProcess | undefined;
  private startPromise: Promise<RpcServerReadyState> | undefined;
  private resolveStart: ((value: RpcServerReadyState) => void) | undefined;
  private rejectStart: ((error: Error) => void) | undefined;
  private readyState: RpcServerReadyState | undefined;
  private stdoutBuffer = "";
  private stderrTail = "";
  private initializeRequestId = "";
  private intentionalStop = false;
  private currentStatus: RpcServerStatus = "stopped";
  private nextRequestId = 1;

  constructor(options: RpcServerManagerOptions) {
    this.launch = options.launch;
    this.workspace = options.workspace;
    this.extensionVersion = options.extensionVersion;
    this.processFactory = options.processFactory ?? nodeRpcProcessFactory;
    this.notifier = options.notifier;
  }

  get status(): RpcServerStatus {
    return this.currentStatus;
  }

  get stderrPreview(): string {
    return this.stderrTail;
  }

  get autoStart(): boolean {
    return this.launch.autoStart;
  }

  start(): Promise<RpcServerReadyState> {
    if (this.readyState !== undefined && this.currentStatus === "ready") {
      return Promise.resolve(this.readyState);
    }

    if (this.startPromise !== undefined) {
      return this.startPromise;
    }

    if (!this.workspace.trusted) {
      this.currentStatus = "failed";
      return Promise.reject(new Error("Workspace is not trusted; RPC server was not started."));
    }

    this.currentStatus = "starting";
    this.intentionalStop = false;
    this.stdoutBuffer = "";
    this.stderrTail = "";
    this.initializeRequestId = `vscode_initialize_${this.nextRequestId}`;
    this.nextRequestId += 1;

    let child: RpcChildProcess;
    try {
      child = this.processFactory.spawn(this.launch.command, this.launch.args, {
        cwd: this.workspace.root,
      });
    } catch (error) {
      const spawnError = asError(error);
      this.currentStatus = "failed";
      return Promise.reject(spawnError);
    }

    if (child.stdin === null || child.stdout === null || child.stderr === null) {
      this.currentStatus = "failed";
      return Promise.reject(new Error("RPC server process did not expose stdio pipes."));
    }

    this.child = child;
    child.stdout.on("data", (chunk) => this.handleStdoutData(chunk));
    child.stderr.on("data", (chunk) => this.handleStderrData(chunk));
    child.on("exit", (code, signal) => this.handleExit(code, signal));
    child.on("error", (error) => this.handleProcessError(error));

    this.startPromise = new Promise<RpcServerReadyState>((resolve, reject) => {
      this.resolveStart = resolve;
      this.rejectStart = reject;
    });

    try {
      child.stdin.write(`${JSON.stringify(this.initializeRequest())}\n`);
    } catch (error) {
      this.failStarting(asError(error));
    }

    return this.startPromise;
  }

  onEvent(handler: (event: AgentEventEnvelope) => void): DisposableLike {
    this.eventHandlers.add(handler);
    return {
      dispose: () => {
        this.eventHandlers.delete(handler);
      },
    };
  }

  async sendRequest<TResult = unknown>(method: string, params?: unknown): Promise<TResult> {
    await this.start();

    const child = this.child;
    if (child === undefined || child.stdin === null || this.currentStatus !== "ready") {
      throw new Error("RPC server is not ready to accept requests.");
    }

    const id = `vscode_request_${this.nextRequestId}`;
    this.nextRequestId += 1;
    const request = {
      jsonrpc: "2.0",
      id,
      method,
      ...(params === undefined ? {} : { params }),
    };

    const promise = new Promise<TResult>((resolve, reject) => {
      this.pendingRequests.set(id, {
        resolve: resolve as (value: unknown) => void,
        reject,
      });
    });

    try {
      child.stdin.write(`${JSON.stringify(request)}\n`);
    } catch (error) {
      this.pendingRequests.delete(id);
      throw asError(error);
    }

    return promise;
  }

  stop(): void {
    this.intentionalStop = true;
    this.readyState = undefined;
    this.clearPendingStart(new Error("RPC server was stopped before initialization completed."));
    this.rejectPendingRequests(new Error("RPC server was stopped before the request completed."));

    const child = this.child;
    this.child = undefined;
    if (child !== undefined && child.killed !== true) {
      child.stdin?.end?.();
      child.kill();
    }

    this.currentStatus = "stopped";
  }

  dispose(): void {
    this.stop();
  }

  private initializeRequest(): unknown {
    return {
      jsonrpc: "2.0",
      id: this.initializeRequestId,
      method: RPC_INITIALIZE_METHOD,
      params: {
        protocolVersion: RPC_PROTOCOL_VERSION,
        client: {
          name: "prole-coder-vscode",
          version: this.extensionVersion,
          frontend: "vscode",
        },
        workspaceRoot: this.workspace.root,
        workspaceTrusted: this.workspace.trusted,
      },
    };
  }

  private handleStdoutData(chunk: Buffer | string): void {
    this.stdoutBuffer += chunk.toString();

    for (;;) {
      const newlineIndex = this.stdoutBuffer.indexOf("\n");
      if (newlineIndex < 0) {
        return;
      }

      const line = this.stdoutBuffer.slice(0, newlineIndex).replace(/\r$/, "");
      this.stdoutBuffer = this.stdoutBuffer.slice(newlineIndex + 1);
      if (line.trim().length > 0) {
        this.handleStdoutLine(line);
      }
    }
  }

  private handleStdoutLine(line: string): void {
    let message: unknown;
    try {
      message = JSON.parse(line);
    } catch (error) {
      this.failStarting(new Error(`RPC server emitted invalid JSON: ${asError(error).message}`));
      return;
    }

    if (isJsonRpcResponse(message) && message.id === this.initializeRequestId) {
      this.handleInitializeResponse(message);
      return;
    }

    if (isJsonRpcResponse(message)) {
      this.handleRequestResponse(message);
      return;
    }

    if (isJsonRpcNotification(message) && message.method === RPC_EVENT_METHOD) {
      if (isAgentEventEnvelope(message.params)) {
        for (const handler of this.eventHandlers) {
          handler(message.params);
        }
      }
    }
  }

  private handleInitializeResponse(message: JsonRpcResponse): void {
    if (message.error !== undefined) {
      this.failStarting(
        new Error(`RPC initialize failed: ${message.error.message} (${message.error.code})`),
      );
      return;
    }

    if (!isRpcServerReadyState(message.result)) {
      this.failStarting(new Error("RPC initialize returned an invalid result shape."));
      return;
    }

    this.readyState = message.result;
    this.currentStatus = "ready";
    const resolve = this.resolveStart;
    this.resolveStart = undefined;
    this.rejectStart = undefined;
    this.startPromise = undefined;
    resolve?.(message.result);
  }

  private handleRequestResponse(message: JsonRpcResponse): void {
    const id = String(message.id);
    const pending = this.pendingRequests.get(id);
    if (pending === undefined) {
      return;
    }

    this.pendingRequests.delete(id);
    if (message.error !== undefined) {
      pending.reject(new RpcRequestError(message.error));
      return;
    }

    pending.resolve(message.result);
  }

  private handleStderrData(chunk: Buffer | string): void {
    this.stderrTail = `${this.stderrTail}${chunk.toString()}`.slice(-4096);
  }

  private handleExit(code: number | null, signal: NodeJS.Signals | null): void {
    const wasIntentional = this.intentionalStop;
    this.child = undefined;
    this.readyState = undefined;
    this.rejectPendingRequests(
      new Error(`RPC server exited before the request completed: ${formatExit(code, signal)}`),
    );

    if (this.currentStatus === "starting") {
      this.failStarting(new Error(`RPC server exited during startup: ${formatExit(code, signal)}`));
      return;
    }

    if (wasIntentional) {
      if (this.currentStatus !== "failed") {
        this.currentStatus = "stopped";
      }
      return;
    }

    if (this.currentStatus === "ready") {
      this.currentStatus = "failed";
      this.notifier?.warn(`prole-coder RPC server exited unexpectedly: ${formatExit(code, signal)}`);
      return;
    }

    if (this.currentStatus !== "stopped") {
      this.currentStatus = "failed";
    }
  }

  private handleProcessError(error: Error): void {
    if (this.currentStatus === "starting") {
      this.failStarting(error);
      return;
    }

    this.readyState = undefined;
    this.currentStatus = "failed";
    this.rejectPendingRequests(error);
    this.notifier?.warn(`prole-coder RPC server error: ${error.message}`);
  }

  private failStarting(error: Error): void {
    const reject = this.rejectStart;
    this.resolveStart = undefined;
    this.rejectStart = undefined;
    this.startPromise = undefined;
    this.readyState = undefined;
    this.currentStatus = "failed";
    this.rejectPendingRequests(error);

    const child = this.child;
    this.child = undefined;
    this.intentionalStop = true;
    if (child !== undefined && child.killed !== true) {
      child.stdin?.end?.();
      child.kill();
    }

    reject?.(error);
  }

  private clearPendingStart(error: Error): void {
    const reject = this.rejectStart;
    this.resolveStart = undefined;
    this.rejectStart = undefined;
    this.startPromise = undefined;
    reject?.(error);
  }

  private rejectPendingRequests(error: Error): void {
    for (const pending of this.pendingRequests.values()) {
      pending.reject(error);
    }
    this.pendingRequests.clear();
  }
}

function isJsonRpcResponse(value: unknown): value is JsonRpcResponse {
  return isRecord(value) && value["jsonrpc"] === "2.0" && "id" in value;
}

function isJsonRpcNotification(value: unknown): value is JsonRpcNotification {
  return isRecord(value) && value["jsonrpc"] === "2.0" && typeof value["method"] === "string";
}

function isRpcServerReadyState(value: unknown): value is RpcServerReadyState {
  return (
    isRecord(value) &&
    typeof value["protocolVersion"] === "string" &&
    isRecord(value["server"]) &&
    typeof value["server"]["name"] === "string" &&
    typeof value["server"]["version"] === "string" &&
    typeof value["stateDir"] === "string"
  );
}

function isAgentEventEnvelope(value: unknown): value is AgentEventEnvelope {
  return (
    isRecord(value) &&
    typeof value["seq"] === "number" &&
    typeof value["time"] === "string" &&
    typeof value["type"] === "string" &&
    typeof value["runId"] === "string" &&
    typeof value["payload"] !== "undefined"
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function asError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}

function formatExit(code: number | null, signal: NodeJS.Signals | null): string {
  if (signal !== null) {
    return `signal ${signal}`;
  }

  return `exit code ${code ?? "unknown"}`;
}
