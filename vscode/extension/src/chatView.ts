import { randomUUID } from "node:crypto";
import * as vscode from "vscode";

import type { SendTurnParams, SendTurnResult } from "@prole-coder/protocol" with {
  "resolution-mode": "import",
};

import { CHAT_RUN_MODES, DEFAULT_CHAT_MODE, parseChatTurnSubmission, sendTurnParams } from "./chatInput";
import { ChatEventTimeline, type ChatTimelineSnapshot } from "./chatEvents";
import type { AgentEventEnvelope, DisposableLike } from "./rpcServer";

export const CHAT_VIEW_ID = "prole-coder.chat";

export interface ChatRpcEventSource {
  onEvent(handler: (event: AgentEventEnvelope) => void): DisposableLike;
}

export interface ChatTurnSender {
  sendTurn(params: SendTurnParams): Promise<SendTurnResult>;
}

export type ChatRpcClient = ChatRpcEventSource & ChatTurnSender;

interface SnapshotWebviewMessage {
  readonly type: "snapshot";
  readonly snapshot: ChatTimelineSnapshot;
}

interface SubmissionWebviewMessage {
  readonly type: "submission";
  readonly submission: ChatSubmissionSnapshot;
}

type ExtensionToWebviewMessage = SnapshotWebviewMessage | SubmissionWebviewMessage;

type ChatSubmissionStatus = "idle" | "sending" | "running" | "completed" | "failed" | "canceled";
type TerminalSubmissionStatus = Extract<ChatSubmissionStatus, "completed" | "failed" | "canceled">;

interface ChatSubmissionSnapshot {
  readonly busy: boolean;
  readonly status: ChatSubmissionStatus;
  readonly message: string;
  readonly runId?: string;
  readonly turnId?: string;
  readonly error?: string;
}

interface TerminalRunState {
  readonly status: TerminalSubmissionStatus;
  readonly message: string;
  readonly error?: string;
}

export class ProleChatViewProvider implements vscode.WebviewViewProvider, DisposableLike {
  private readonly timeline = new ChatEventTimeline();
  private readonly terminalRuns = new Map<string, TerminalRunState>();
  private readonly rpcClient: ChatRpcClient | undefined;
  private submission: ChatSubmissionSnapshot = idleSubmission();
  private rpcSubscription: DisposableLike | undefined;
  private viewMessageSubscription: DisposableLike | undefined;
  private view: vscode.WebviewView | undefined;

  constructor(
    private readonly extensionUri: vscode.Uri,
    rpcClient?: ChatRpcClient,
  ) {
    this.rpcClient = rpcClient;
    this.rpcSubscription = rpcClient?.onEvent((event) => {
      this.timeline.append(event);
      this.updateSubmissionForEvent(event);
      this.postSnapshot();
      this.postSubmission();
    });
  }

  resolveWebviewView(webviewView: vscode.WebviewView): void {
    this.view = webviewView;
    webviewView.webview.options = {
      enableScripts: true,
      localResourceRoots: [this.extensionUri],
    };
    this.viewMessageSubscription?.dispose();
    this.viewMessageSubscription = webviewView.webview.onDidReceiveMessage((message) => {
      void this.handleWebviewMessage(message);
    });
    webviewView.webview.html = renderChatViewHtml(
      webviewView.webview,
      this.timeline.snapshot(),
      this.submission,
    );
    this.postSnapshot();
    this.postSubmission();
  }

  openChatView(): Thenable<unknown> {
    return vscode.commands.executeCommand(`${CHAT_VIEW_ID}.focus`);
  }

  dispose(): void {
    this.rpcSubscription?.dispose();
    this.viewMessageSubscription?.dispose();
    this.rpcSubscription = undefined;
    this.viewMessageSubscription = undefined;
  }

  private async handleWebviewMessage(message: unknown): Promise<void> {
    if (!isRecord(message) || message["type"] !== "submitTurn") {
      return;
    }

    const parsed = parseChatTurnSubmission(message);
    if (!parsed.ok) {
      this.setSubmission({
        ...idleSubmission(),
        status: "failed",
        message: parsed.error,
        error: parsed.error,
      });
      return;
    }

    if (this.rpcClient === undefined) {
      this.setSubmission({
        ...idleSubmission(),
        status: "failed",
        message: "Open a trusted workspace before sending a turn.",
        error: "No trusted workspace is available.",
      });
      return;
    }

    if (this.submission.busy) {
      this.setSubmission({
        ...this.submission,
        message: "A turn is already running.",
      });
      return;
    }

    this.setSubmission({
      busy: true,
      status: "sending",
      message: "Sending turn...",
    });

    try {
      const result = await this.rpcClient.sendTurn(sendTurnParams(parsed.value));
      const terminal = this.terminalRuns.get(result.runId);
      this.setSubmission(
        terminal === undefined
          ? {
              busy: true,
              status: "running",
              message: "Agent turn running.",
              runId: result.runId,
              turnId: result.turnId,
            }
          : terminalSubmission(result.runId, result.turnId, terminal),
      );
    } catch (error) {
      const messageText = `Failed to send turn: ${errorMessage(error)}`;
      this.setSubmission({
        ...idleSubmission(),
        status: "failed",
        message: messageText,
        error: messageText,
      });
    }
  }

  private postSnapshot(): void {
    const message: ExtensionToWebviewMessage = {
      type: "snapshot",
      snapshot: this.timeline.snapshot(),
    };
    void this.view?.webview.postMessage(message);
  }

  private postSubmission(): void {
    const message: ExtensionToWebviewMessage = {
      type: "submission",
      submission: this.submission,
    };
    void this.view?.webview.postMessage(message);
  }

  private setSubmission(submission: ChatSubmissionSnapshot): void {
    this.submission = submission;
    this.postSubmission();
  }

  private updateSubmissionForEvent(event: AgentEventEnvelope): void {
    const terminal = terminalRunState(event);
    if (terminal === undefined) {
      return;
    }

    this.rememberTerminalRun(event.runId, terminal);
    if (this.submission.runId === undefined || event.runId !== this.submission.runId) {
      return;
    }

    this.submission = terminalSubmission(event.runId, this.submission.turnId, terminal);
  }

  private rememberTerminalRun(runId: string, terminal: TerminalRunState): void {
    this.terminalRuns.set(runId, terminal);
    while (this.terminalRuns.size > 50) {
      const oldest = this.terminalRuns.keys().next().value;
      if (oldest === undefined) {
        return;
      }
      this.terminalRuns.delete(oldest);
    }
  }
}

function renderChatViewHtml(
  webview: vscode.Webview,
  snapshot: ChatTimelineSnapshot,
  submission: ChatSubmissionSnapshot,
): string {
  const nonce = nonceValue();
  const initialSnapshot = safeScriptJson(snapshot);
  const initialSubmission = safeScriptJson(submission);
  return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource} 'nonce-${nonce}'; script-src 'nonce-${nonce}';">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <style nonce="${nonce}">
    :root {
      color-scheme: light dark;
    }

    body {
      margin: 0;
      padding: 0;
      color: var(--vscode-foreground);
      background: var(--vscode-sideBar-background);
      font: var(--vscode-font-size) var(--vscode-font-family);
    }

    .shell {
      display: flex;
      min-height: 100vh;
      flex-direction: column;
    }

    .status {
      display: grid;
      gap: 2px;
      padding: 10px 12px;
      border-bottom: 1px solid var(--vscode-sideBarSectionHeader-border);
      background: var(--vscode-sideBarSectionHeader-background);
    }

    .status-title {
      font-weight: 600;
    }

    .status-subtitle {
      color: var(--vscode-descriptionForeground);
      font-size: 12px;
      white-space: nowrap;
      overflow: hidden;
      text-overflow: ellipsis;
    }

    .events {
      display: grid;
      gap: 8px;
      padding: 10px;
      overflow: auto;
    }

    .empty {
      padding: 20px 12px;
      color: var(--vscode-descriptionForeground);
      text-align: center;
    }

    .item {
      display: grid;
      gap: 6px;
      padding: 8px;
      border: 1px solid var(--vscode-editorWidget-border);
      border-left-width: 3px;
      background: var(--vscode-editorWidget-background);
    }

    .item.assistant {
      border-radius: 6px;
      background: var(--vscode-input-background);
    }

    .item.running {
      border-left-color: var(--vscode-progressBar-background);
    }

    .item.success {
      border-left-color: var(--vscode-testing-iconPassed);
    }

    .item.warning {
      border-left-color: var(--vscode-editorWarning-foreground);
    }

    .item.danger {
      border-left-color: var(--vscode-editorError-foreground);
    }

    .meta {
      display: flex;
      align-items: center;
      gap: 6px;
      color: var(--vscode-descriptionForeground);
      font-size: 11px;
      min-width: 0;
    }

    .seq {
      flex: 0 0 auto;
      font-variant-numeric: tabular-nums;
    }

    .type {
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .title {
      font-weight: 600;
      line-height: 1.35;
    }

    .body {
      white-space: pre-wrap;
      line-height: 1.4;
      overflow-wrap: anywhere;
    }
  </style>
</head>
<body>
  <main class="shell">
    <section class="status" aria-live="polite">
      <div id="status-title" class="status-title">ProleCoder</div>
      <div id="status-subtitle" class="status-subtitle">No run events yet.</div>
    </section>
    <section id="events" class="events" aria-label="Run events"></section>
  </main>
  <script nonce="${nonce}">
    const initialSnapshot = ${initialSnapshot};
    const eventsRoot = document.getElementById("events");
    const statusTitle = document.getElementById("status-title");
    const statusSubtitle = document.getElementById("status-subtitle");

    window.addEventListener("message", (event) => {
      const message = event.data;
      if (message && message.type === "snapshot") {
        render(message.snapshot);
      }
    });

    render(initialSnapshot);

    function render(snapshot) {
      const items = Array.isArray(snapshot.items) ? snapshot.items : [];
      statusTitle.textContent = snapshot.latestStatus || "ProleCoder";
      statusSubtitle.textContent = snapshot.latestRunId
        ? snapshot.latestRunId + " · " + snapshot.eventCount + " events"
        : "No run events yet.";
      eventsRoot.replaceChildren();

      if (items.length === 0) {
        const empty = document.createElement("div");
        empty.className = "empty";
        empty.textContent = "No run events yet.";
        eventsRoot.append(empty);
        return;
      }

      for (const item of items) {
        eventsRoot.append(renderItem(item));
      }
    }

    function renderItem(item) {
      const article = document.createElement("article");
      article.className = "item " + item.kind + " " + item.tone;

      const meta = document.createElement("div");
      meta.className = "meta";
      const seq = document.createElement("span");
      seq.className = "seq";
      seq.textContent = item.seq === item.lastSeq ? "#" + item.seq : "#" + item.seq + "-" + item.lastSeq;
      const type = document.createElement("span");
      type.className = "type";
      type.textContent = item.type;
      meta.append(seq, type);

      const title = document.createElement("div");
      title.className = "title";
      title.textContent = item.title;

      article.append(meta, title);
      if (item.body) {
        const body = document.createElement("div");
        body.className = "body";
        body.textContent = item.body;
        article.append(body);
      }

      return article;
    }
  </script>
</body>
</html>`;
}

function safeScriptJson(value: unknown): string {
  return JSON.stringify(value).replaceAll("<", "\\u003c");
}

function nonceValue(): string {
  return randomUUID().replaceAll("-", "");
}

function idleSubmission(): ChatSubmissionSnapshot {
  return {
    busy: false,
    status: "idle",
    message: "",
  };
}

function terminalSubmission(
  runId: string,
  turnId: string | undefined,
  terminal: TerminalRunState,
): ChatSubmissionSnapshot {
  return {
    busy: false,
    status: terminal.status,
    message: terminal.message,
    ...(terminal.error === undefined ? {} : { error: terminal.error }),
    runId,
    ...(turnId === undefined ? {} : { turnId }),
  };
}

function terminalRunState(event: AgentEventEnvelope): TerminalRunState | undefined {
  if (event.type === "run.completed") {
    return {
      status: "completed",
      message: "Run completed.",
    };
  }

  if (event.type === "run.failed") {
    const message = terminalMessage(event, "Run failed.");
    return {
      status: "failed",
      message,
      error: message,
    };
  }

  if (event.type === "run.canceled") {
    return {
      status: "canceled",
      message: terminalMessage(event, "Run canceled."),
    };
  }

  return undefined;
}

function terminalMessage(event: AgentEventEnvelope, fallback: string): string {
  const payload = isRecord(event.payload) ? event.payload : undefined;
  const message = payload?.["message"] ?? payload?.["reason"] ?? payload?.["summary"];
  return typeof message === "string" && message.length > 0 ? message : fallback;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
