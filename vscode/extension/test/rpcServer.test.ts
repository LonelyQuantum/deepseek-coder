import assert from "node:assert/strict";
import test from "node:test";

import {
  DEFAULT_RPC_ARGS,
  DEFAULT_RPC_COMMAND,
  RPC_INITIALIZE_METHOD,
  RPC_PROTOCOL_VERSION,
  RpcRequestError,
  RpcServerManager,
  type RpcChildProcess,
  type RpcProcessFactory,
  type RpcReadable,
  type RpcServerConfiguration,
  type RpcSpawnOptions,
  type RpcWritable,
  readRpcServerLaunchConfig,
} from "../src/rpcServer.js";

test("RPC server manager spawns the configured command and initializes the workspace", async () => {
  const factory = new FakeProcessFactory();
  const manager = new RpcServerManager({
    launch: {
      command: "prole",
      args: ["rpc", "--provider", "fixture"],
      autoStart: true,
    },
    workspace: {
      root: "C:/workspace/project",
      trusted: true,
    },
    extensionVersion: "0.1.0",
    processFactory: factory,
  });

  const readyPromise = manager.start();
  const child = factory.lastChild();
  const request = child.initializeRequest();

  assert.equal(factory.lastCommand, "prole");
  assert.deepEqual(factory.lastArgs, ["rpc", "--provider", "fixture"]);
  assert.equal(factory.lastOptions?.cwd, "C:/workspace/project");
  assert.equal(request.method, RPC_INITIALIZE_METHOD);
  assert.equal(request.params.protocolVersion, RPC_PROTOCOL_VERSION);
  assert.equal(request.params.client.frontend, "vscode");
  assert.equal(request.params.workspaceRoot, "C:/workspace/project");
  assert.equal(request.params.workspaceTrusted, true);

  child.stdout.pushJson(initializeResponse(request.id));
  const ready = await readyPromise;

  assert.equal(manager.status, "ready");
  assert.equal(ready.server.name, "prole-coder-agent-rpc");
});

test("RPC server manager forwards agent.event notifications", async () => {
  const factory = new FakeProcessFactory();
  const manager = rpcManagerWithFactory(factory);
  const received: unknown[] = [];
  manager.onEvent((event) => received.push(event));

  const readyPromise = manager.start();
  const child = factory.lastChild();
  child.stdout.pushJson(initializeResponse(child.initializeRequest().id));
  await readyPromise;

  child.stdout.pushJson({
    jsonrpc: "2.0",
    method: "agent.event",
    params: {
      seq: 1,
      time: "1970-01-01T00:00:00.000Z",
      type: "run.started",
      runId: "run_1",
      payload: { mode: "ask" },
    },
  });

  assert.deepEqual(received, [
    {
      seq: 1,
      time: "1970-01-01T00:00:00.000Z",
      type: "run.started",
      runId: "run_1",
      payload: { mode: "ask" },
    },
  ]);
});

test("RPC server manager ignores malformed agent.event notifications", async () => {
  const factory = new FakeProcessFactory();
  const manager = rpcManagerWithFactory(factory);
  const received: unknown[] = [];
  manager.onEvent((event) => received.push(event));

  const readyPromise = manager.start();
  const child = factory.lastChild();
  child.stdout.pushJson(initializeResponse(child.initializeRequest().id));
  await readyPromise;

  child.stdout.pushJson({
    jsonrpc: "2.0",
    method: "agent.event",
    params: {
      seq: 1,
      time: "1970-01-01T00:00:00.000Z",
      type: "run.started",
      payload: { mode: "ask" },
    },
  });

  assert.deepEqual(received, []);
});

test("RPC server manager sends JSON-RPC requests and resolves matching responses", async () => {
  const factory = new FakeProcessFactory();
  const manager = rpcManagerWithFactory(factory);
  const readyPromise = manager.start();
  const child = factory.lastChild();
  child.stdout.pushJson(initializeResponse(child.initializeRequest().id));
  await readyPromise;

  const responsePromise = manager.sendRequest<{ accepted: boolean }>("agent.sendTurn", {
    prompt: "hello",
  });
  await flushMicrotasks();
  const request = child.requestAt(1);

  assert.equal(request.method, "agent.sendTurn");
  assert.deepEqual(request.params, { prompt: "hello" });

  child.stdout.pushJson({
    jsonrpc: "2.0",
    id: request.id,
    result: { accepted: true },
  });

  assert.deepEqual(await responsePromise, { accepted: true });
});

test("RPC server manager rejects JSON-RPC error responses", async () => {
  const factory = new FakeProcessFactory();
  const manager = rpcManagerWithFactory(factory);
  const readyPromise = manager.start();
  const child = factory.lastChild();
  child.stdout.pushJson(initializeResponse(child.initializeRequest().id));
  await readyPromise;

  const responsePromise = manager.sendRequest("agent.approve", {
    approvalId: "approval_1",
  });
  await flushMicrotasks();
  const request = child.requestAt(1);

  child.stdout.pushJson({
    jsonrpc: "2.0",
    id: request.id,
    error: {
      code: -32003,
      message: "approval not found",
      data: { approvalId: "approval_1" },
    },
  });

  await assert.rejects(responsePromise, (error: unknown) => {
    assert.ok(error instanceof RpcRequestError);
    assert.equal(error.code, -32003);
    assert.deepEqual(error.data, { approvalId: "approval_1" });
    return true;
  });
});

test("RPC server manager rejects pending requests when the server exits", async () => {
  const factory = new FakeProcessFactory();
  const manager = rpcManagerWithFactory(factory);
  const readyPromise = manager.start();
  const child = factory.lastChild();
  child.stdout.pushJson(initializeResponse(child.initializeRequest().id));
  await readyPromise;

  const responsePromise = manager.sendRequest("agent.sendTurn", { prompt: "hello" });
  await flushMicrotasks();

  child.exit(1, null);

  await assert.rejects(responsePromise, /exited before the request completed/);
});

test("RPC server manager rejects untrusted workspaces without spawning", async () => {
  const factory = new FakeProcessFactory();
  const manager = new RpcServerManager({
    launch: {
      command: "prole",
      args: ["rpc"],
      autoStart: true,
    },
    workspace: {
      root: "C:/workspace/project",
      trusted: false,
    },
    extensionVersion: "0.1.0",
    processFactory: factory,
  });

  await assert.rejects(manager.start(), /not trusted/);

  assert.equal(factory.spawnCount, 0);
  assert.equal(manager.status, "failed");
});

test("RPC server manager warns when a ready server exits unexpectedly", async () => {
  const factory = new FakeProcessFactory();
  const warnings: string[] = [];
  const manager = new RpcServerManager({
    launch: {
      command: "prole",
      args: ["rpc"],
      autoStart: true,
    },
    workspace: {
      root: "C:/workspace/project",
      trusted: true,
    },
    extensionVersion: "0.1.0",
    processFactory: factory,
    notifier: {
      info: () => undefined,
      warn(message) {
        warnings.push(message);
      },
    },
  });

  const readyPromise = manager.start();
  const child = factory.lastChild();
  child.stdout.pushJson(initializeResponse(child.initializeRequest().id));
  await readyPromise;

  child.exit(1, null);

  assert.equal(manager.status, "failed");
  assert.equal(warnings.length, 1);
  assert.ok(warnings[0]?.includes("exit code 1"));
});

test("RPC server manager disposes the child process", async () => {
  const factory = new FakeProcessFactory();
  const manager = rpcManagerWithFactory(factory);
  const readyPromise = manager.start();
  const child = factory.lastChild();
  child.stdout.pushJson(initializeResponse(child.initializeRequest().id));
  await readyPromise;

  manager.dispose();

  assert.equal(child.killed, true);
  assert.equal(child.stdin.ended, true);
  assert.equal(manager.status, "stopped");
});

test("RPC launch configuration reads defaults and rejects an empty command", () => {
  const defaults = readRpcServerLaunchConfig(new FakeConfiguration({}));
  assert.equal(defaults.command, DEFAULT_RPC_COMMAND);
  assert.deepEqual(defaults.args, [...DEFAULT_RPC_ARGS]);
  assert.equal(defaults.autoStart, true);

  const custom = readRpcServerLaunchConfig(
    new FakeConfiguration({
      command: "cargo",
      args: ["run", "-p", "prole-coder-cli", "--", "rpc"],
      autoStart: false,
    }),
  );
  assert.equal(custom.command, "cargo");
  assert.deepEqual(custom.args, ["run", "-p", "prole-coder-cli", "--", "rpc"]);
  assert.equal(custom.autoStart, false);

  assert.throws(() => readRpcServerLaunchConfig(new FakeConfiguration({ command: " " })));
});

function rpcManagerWithFactory(factory: FakeProcessFactory): RpcServerManager {
  return new RpcServerManager({
    launch: {
      command: "prole",
      args: ["rpc"],
      autoStart: true,
    },
    workspace: {
      root: "C:/workspace/project",
      trusted: true,
    },
    extensionVersion: "0.1.0",
    processFactory: factory,
  });
}

function initializeResponse(id: unknown): unknown {
  return {
    jsonrpc: "2.0",
    id,
    result: {
      protocolVersion: RPC_PROTOCOL_VERSION,
      server: {
        name: "prole-coder-agent-rpc",
        version: "0.1.0",
      },
      capabilities: {
        protocolVersion: RPC_PROTOCOL_VERSION,
        supportsRunResume: true,
        supportsPatchApproval: true,
        supportsPersistentApprovals: false,
        supportedRiskLevels: ["read", "write", "exec", "network", "destructive"],
      },
      stateDir: ".prole-coder",
    },
  };
}

class FakeConfiguration implements RpcServerConfiguration {
  constructor(private readonly values: Record<string, unknown>) {}

  get<T>(section: string, defaultValue: T): T {
    return section in this.values ? (this.values[section] as T) : defaultValue;
  }
}

class FakeProcessFactory implements RpcProcessFactory {
  lastCommand: string | undefined;
  lastArgs: readonly string[] | undefined;
  lastOptions: RpcSpawnOptions | undefined;
  spawnCount = 0;
  private child: FakeChildProcess | undefined;

  spawn(command: string, args: readonly string[], options: RpcSpawnOptions): RpcChildProcess {
    this.spawnCount += 1;
    this.lastCommand = command;
    this.lastArgs = [...args];
    this.lastOptions = options;
    this.child = new FakeChildProcess();
    return this.child;
  }

  lastChild(): FakeChildProcess {
    assert.ok(this.child);
    return this.child;
  }
}

class FakeWritable implements RpcWritable {
  readonly writes: string[] = [];
  ended = false;

  write(data: string): unknown {
    this.writes.push(data);
    return true;
  }

  end(): unknown {
    this.ended = true;
    return undefined;
  }
}

class FakeReadable implements RpcReadable {
  private readonly dataListeners: Array<(chunk: Buffer | string) => void> = [];

  on(event: "data", listener: (chunk: Buffer | string) => void): unknown {
    if (event === "data") {
      this.dataListeners.push(listener);
    }
    return this;
  }

  pushJson(value: unknown): void {
    this.push(`${JSON.stringify(value)}\n`);
  }

  push(chunk: string): void {
    for (const listener of this.dataListeners) {
      listener(chunk);
    }
  }
}

class FakeChildProcess implements RpcChildProcess {
  readonly stdin = new FakeWritable();
  readonly stdout = new FakeReadable();
  readonly stderr = new FakeReadable();
  killed = false;

  private readonly exitListeners: Array<(code: number | null, signal: NodeJS.Signals | null) => void> =
    [];
  private readonly errorListeners: Array<(error: Error) => void> = [];

  kill(): boolean {
    this.killed = true;
    return true;
  }

  on(
    event: "exit" | "error",
    listener:
      | ((code: number | null, signal: NodeJS.Signals | null) => void)
      | ((error: Error) => void),
  ): unknown {
    if (event === "exit") {
      this.exitListeners.push(listener as (code: number | null, signal: NodeJS.Signals | null) => void);
    } else {
      this.errorListeners.push(listener as (error: Error) => void);
    }
    return this;
  }

  initializeRequest(): InitializeRequest {
    const firstWrite = this.stdin.writes[0];
    assert.ok(firstWrite);
    return JSON.parse(firstWrite) as InitializeRequest;
  }

  exit(code: number | null, signal: NodeJS.Signals | null): void {
    for (const listener of this.exitListeners) {
      listener(code, signal);
    }
  }

  error(error: Error): void {
    for (const listener of this.errorListeners) {
      listener(error);
    }
  }

  requestAt(index: number): RpcRequest {
    const write = this.stdin.writes[index];
    assert.ok(write);
    return JSON.parse(write) as RpcRequest;
  }
}

interface InitializeRequest {
  readonly id: string;
  readonly method: string;
  readonly params: {
    readonly protocolVersion: string;
    readonly client: {
      readonly frontend: string;
    };
    readonly workspaceRoot: string;
    readonly workspaceTrusted: boolean;
  };
}

interface RpcRequest {
  readonly id: string;
  readonly method: string;
  readonly params?: unknown;
}

async function flushMicrotasks(): Promise<void> {
  await Promise.resolve();
}
