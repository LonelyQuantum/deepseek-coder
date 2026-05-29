import { randomUUID } from "node:crypto";
import * as vscode from "vscode";

import type {
  ListRunsParams,
  ListRunsResult,
  ResumeParams,
  ResumeResult,
  SendTurnParams,
  SendTurnResult,
} from "@prole-coder/protocol" with {
  "resolution-mode": "import",
};

import { CHAT_RUN_MODES, DEFAULT_CHAT_MODE, parseChatTurnSubmission, sendTurnParams } from "./chatInput";
import { ChatEventTimeline, type ChatTimelineSnapshot } from "./chatEvents";
import type { AgentEventEnvelope, DisposableLike } from "./rpcServer";
import {
  RUN_LIST_LIMIT,
  failedRunList,
  idleRunList,
  isRefreshRunsMessage,
  loadingRunList,
  readyRunList,
  resumeRunIdFromMessage,
  type RunListSnapshot,
} from "./runHistory";

export const CHAT_VIEW_ID = "prole-coder.chat";

export interface ChatRpcEventSource {
  onEvent(handler: (event: AgentEventEnvelope) => void): DisposableLike;
}

export interface ChatTurnSender {
  sendTurn(params: SendTurnParams): Promise<SendTurnResult>;
}

export interface ChatRunHistoryClient {
  listRuns(params?: ListRunsParams): Promise<ListRunsResult>;
  resume(params: ResumeParams): Promise<ResumeResult>;
}

export type ChatRpcClient = ChatRpcEventSource & ChatTurnSender & ChatRunHistoryClient;

interface SnapshotWebviewMessage {
  readonly type: "snapshot";
  readonly snapshot: ChatTimelineSnapshot;
}

interface SubmissionWebviewMessage {
  readonly type: "submission";
  readonly submission: ChatSubmissionSnapshot;
}

interface RunsWebviewMessage {
  readonly type: "runs";
  readonly runs: RunListSnapshot;
}

type ExtensionToWebviewMessage = SnapshotWebviewMessage | SubmissionWebviewMessage | RunsWebviewMessage;

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
  private runList: RunListSnapshot = idleRunList();
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
      const terminal = this.updateSubmissionForEvent(event);
      this.postSnapshot();
      this.postSubmission();
      if (terminal && this.view !== undefined) {
        void this.refreshRuns("Refreshing runs...");
      }
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
      this.runList,
    );
    this.postSnapshot();
    this.postSubmission();
    this.postRuns();
    void this.refreshRuns();
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
    if (isRefreshRunsMessage(message)) {
      await this.refreshRuns();
      return;
    }

    const resumeRunId = resumeRunIdFromMessage(message);
    if (resumeRunId !== undefined) {
      await this.resumeRun(resumeRunId);
      return;
    }

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
      void this.refreshRuns("Refreshing runs...");
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

  private postRuns(): void {
    const message: ExtensionToWebviewMessage = {
      type: "runs",
      runs: this.runList,
    };
    void this.view?.webview.postMessage(message);
  }

  private setSubmission(submission: ChatSubmissionSnapshot): void {
    this.submission = submission;
    this.postSubmission();
  }

  private setRunList(runList: RunListSnapshot): void {
    this.runList = runList;
    this.postRuns();
  }

  private async refreshRuns(message = "Loading runs..."): Promise<void> {
    if (this.rpcClient === undefined) {
      this.setRunList(
        failedRunList("Open a trusted workspace before loading runs.", this.runList),
      );
      return;
    }

    this.setRunList(loadingRunList(this.runList, message));
    try {
      const result = await this.rpcClient.listRuns({ limit: RUN_LIST_LIMIT });
      this.setRunList(readyRunList(result, this.runList.selectedRunId));
    } catch (error) {
      this.setRunList(failedRunList(`Failed to load runs: ${errorMessage(error)}`, this.runList));
    }
  }

  private async resumeRun(runId: string): Promise<void> {
    if (this.rpcClient === undefined) {
      this.setRunList(
        failedRunList("Open a trusted workspace before replaying a run.", this.runList),
      );
      return;
    }

    if (this.submission.busy) {
      this.setRunList(failedRunList("A turn is already running.", this.runList));
      return;
    }

    this.timeline.clear();
    this.postSnapshot();
    this.setSubmission(idleSubmission());
    this.setRunList(loadingRunList(this.runList, "Replaying run..."));
    try {
      const result = await this.rpcClient.resume({ runId });
      const message = result.replayStarted
        ? `Replaying ${result.runId} through seq ${result.nextSeq - 1}.`
        : `No events to replay for ${result.runId}.`;
      this.setRunList(readyRunList({ runs: this.runList.runs }, result.runId, message));
    } catch (error) {
      this.setRunList(failedRunList(`Failed to resume run: ${errorMessage(error)}`, this.runList));
    }
  }

  private updateSubmissionForEvent(event: AgentEventEnvelope): boolean {
    const terminal = terminalRunState(event);
    if (terminal === undefined) {
      return false;
    }

    this.rememberTerminalRun(event.runId, terminal);
    if (this.submission.runId === undefined || event.runId !== this.submission.runId) {
      return false;
    }

    this.submission = terminalSubmission(event.runId, this.submission.turnId, terminal);
    return true;
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
  runList: RunListSnapshot,
): string {
  const nonce = nonceValue();
  const initialSnapshot = safeScriptJson(snapshot);
  const initialSubmission = safeScriptJson(submission);
  const initialRuns = safeScriptJson(runList);
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

    .runs {
      display: grid;
      gap: 6px;
      padding: 8px 10px;
      border-bottom: 1px solid var(--vscode-sideBarSectionHeader-border);
      background: var(--vscode-sideBar-background);
    }

    .runs-header {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 8px;
      min-width: 0;
    }

    .runs-title {
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      font-weight: 600;
    }

    .refresh-runs {
      flex: 0 0 auto;
      height: 24px;
      padding: 0 8px;
      color: var(--vscode-button-secondaryForeground);
      background: var(--vscode-button-secondaryBackground);
      border: 0;
      font: var(--vscode-font-size) var(--vscode-font-family);
    }

    .refresh-runs:hover:enabled {
      background: var(--vscode-button-secondaryHoverBackground);
    }

    .refresh-runs:disabled {
      opacity: 0.65;
    }

    .run-message {
      min-height: 16px;
      color: var(--vscode-descriptionForeground);
      font-size: 12px;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .run-message.failed {
      color: var(--vscode-editorError-foreground);
    }

    .run-list {
      display: grid;
      gap: 4px;
      max-height: 180px;
      overflow: auto;
    }

    .run-entry {
      display: grid;
      gap: 2px;
      width: 100%;
      min-height: 48px;
      box-sizing: border-box;
      padding: 6px 7px;
      color: var(--vscode-foreground);
      background: transparent;
      border: 1px solid transparent;
      border-left: 3px solid var(--vscode-editorWidget-border);
      font: var(--vscode-font-size) var(--vscode-font-family);
      text-align: left;
    }

    .run-entry:hover:enabled,
    .run-entry.selected {
      background: var(--vscode-list-hoverBackground);
      border-color: var(--vscode-list-focusOutline, var(--vscode-editorWidget-border));
    }

    .run-entry:disabled {
      opacity: 0.7;
    }

    .run-entry.running {
      border-left-color: var(--vscode-progressBar-background);
    }

    .run-entry.completed {
      border-left-color: var(--vscode-testing-iconPassed);
    }

    .run-entry.failed {
      border-left-color: var(--vscode-editorError-foreground);
    }

    .run-entry.canceled {
      border-left-color: var(--vscode-editorWarning-foreground);
    }

    .run-entry-title {
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
      font-weight: 600;
      line-height: 1.3;
    }

    .run-entry-meta {
      display: flex;
      gap: 6px;
      min-width: 0;
      color: var(--vscode-descriptionForeground);
      font-size: 11px;
      line-height: 1.3;
    }

    .run-entry-meta span {
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .events {
      display: grid;
      gap: 8px;
      flex: 1;
      align-content: start;
      min-height: 0;
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

    .composer {
      display: grid;
      gap: 8px;
      padding: 10px;
      border-top: 1px solid var(--vscode-sideBarSectionHeader-border);
      background: var(--vscode-sideBar-background);
    }

    .prompt {
      box-sizing: border-box;
      width: 100%;
      min-height: 72px;
      max-height: 180px;
      resize: vertical;
      padding: 7px 8px;
      color: var(--vscode-input-foreground);
      background: var(--vscode-input-background);
      border: 1px solid var(--vscode-input-border, transparent);
      font: var(--vscode-font-size) var(--vscode-font-family);
      line-height: 1.4;
    }

    .prompt:focus,
    .mode:focus,
    .send:focus,
    .refresh-runs:focus,
    .run-entry:focus {
      outline: 1px solid var(--vscode-focusBorder);
      outline-offset: 1px;
    }

    .composer-row {
      display: flex;
      align-items: center;
      gap: 8px;
      min-width: 0;
    }

    .mode {
      min-width: 92px;
      height: 28px;
      color: var(--vscode-dropdown-foreground);
      background: var(--vscode-dropdown-background);
      border: 1px solid var(--vscode-dropdown-border, transparent);
      font: var(--vscode-font-size) var(--vscode-font-family);
    }

    .send {
      flex: 0 0 auto;
      min-width: 64px;
      height: 28px;
      padding: 0 10px;
      color: var(--vscode-button-foreground);
      background: var(--vscode-button-background);
      border: 0;
      font: var(--vscode-font-size) var(--vscode-font-family);
      font-weight: 600;
    }

    .send:hover:enabled {
      background: var(--vscode-button-hoverBackground);
    }

    .send:disabled,
    .prompt:disabled,
    .mode:disabled {
      opacity: 0.65;
    }

    .submission {
      min-width: 0;
      flex: 1;
      color: var(--vscode-descriptionForeground);
      font-size: 12px;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }

    .submission.failed {
      color: var(--vscode-editorError-foreground);
    }

    .submission.canceled {
      color: var(--vscode-editorWarning-foreground);
    }

    .submission.completed {
      color: var(--vscode-testing-iconPassed);
    }
  </style>
</head>
<body>
  <main class="shell">
    <section class="status" aria-live="polite">
      <div id="status-title" class="status-title">ProleCoder</div>
      <div id="status-subtitle" class="status-subtitle">No run events yet.</div>
    </section>
    <section class="runs" aria-label="Runs">
      <div class="runs-header">
        <div class="runs-title">Runs</div>
        <button id="refresh-runs" class="refresh-runs" type="button">Refresh</button>
      </div>
      <div id="run-message" class="run-message" aria-live="polite"></div>
      <div id="run-list" class="run-list"></div>
    </section>
    <section id="events" class="events" aria-label="Run events"></section>
    <form id="composer" class="composer">
      <textarea id="prompt" class="prompt" rows="3" placeholder="Ask ProleCoder" aria-label="Chat message"></textarea>
      <div class="composer-row">
        <select id="mode" class="mode" aria-label="Run mode"></select>
        <button id="send" class="send" type="submit">Send</button>
        <div id="submission" class="submission" aria-live="polite"></div>
      </div>
    </form>
  </main>
  <script nonce="${nonce}">
    const initialSnapshot = ${initialSnapshot};
    const initialSubmission = ${initialSubmission};
    const initialRuns = ${initialRuns};
    const runModes = ${safeScriptJson(CHAT_RUN_MODES)};
    const defaultMode = ${safeScriptJson(DEFAULT_CHAT_MODE)};
    const vscodeApi = acquireVsCodeApi();
    const eventsRoot = document.getElementById("events");
    const runListRoot = document.getElementById("run-list");
    const runMessageRoot = document.getElementById("run-message");
    const refreshRunsButton = document.getElementById("refresh-runs");
    const statusTitle = document.getElementById("status-title");
    const statusSubtitle = document.getElementById("status-subtitle");
    const composer = document.getElementById("composer");
    const promptInput = document.getElementById("prompt");
    const modeInput = document.getElementById("mode");
    const sendButton = document.getElementById("send");
    const submissionRoot = document.getElementById("submission");

    for (const mode of runModes) {
      const option = document.createElement("option");
      option.value = mode;
      option.textContent = mode[0].toUpperCase() + mode.slice(1);
      modeInput.append(option);
    }
    modeInput.value = defaultMode;

    window.addEventListener("message", (event) => {
      const message = event.data;
      if (message && message.type === "snapshot") {
        render(message.snapshot);
      }
      if (message && message.type === "submission") {
        renderSubmission(message.submission);
      }
      if (message && message.type === "runs") {
        renderRuns(message.runs);
      }
    });

    refreshRunsButton.addEventListener("click", () => {
      vscodeApi.postMessage({ type: "refreshRuns" });
    });

    composer.addEventListener("submit", (event) => {
      event.preventDefault();
      const message = promptInput.value.trim();
      if (!message) {
        renderSubmission({
          busy: false,
          status: "failed",
          message: "Enter a message before sending.",
        });
        promptInput.focus();
        return;
      }

      setComposerBusy(true);
      submissionRoot.className = "submission sending";
      submissionRoot.textContent = "Sending turn...";
      vscodeApi.postMessage({
        type: "submitTurn",
        message,
        mode: modeInput.value,
      });
    });

    promptInput.addEventListener("keydown", (event) => {
      if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
        event.preventDefault();
        composer.requestSubmit();
      }
    });

    render(initialSnapshot);
    renderSubmission(initialSubmission);
    renderRuns(initialRuns);

    function render(snapshot) {
      const items = Array.isArray(snapshot.items) ? snapshot.items : [];
      statusTitle.textContent = snapshot.latestStatus || "ProleCoder";
      statusSubtitle.textContent = snapshot.latestRunId
        ? snapshot.latestRunId + " - " + snapshot.eventCount + " events"
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

    function renderRuns(snapshot) {
      const state = snapshot && typeof snapshot === "object" ? snapshot : initialRuns;
      const runs = Array.isArray(state.runs) ? state.runs : [];
      const status = typeof state.status === "string" ? state.status : "idle";
      const loading = status === "loading";
      refreshRunsButton.disabled = loading;
      runMessageRoot.className = "run-message " + status;
      runMessageRoot.textContent = typeof state.message === "string" ? state.message : "";
      runListRoot.replaceChildren();

      if (runs.length === 0) {
        const empty = document.createElement("div");
        empty.className = "empty";
        empty.textContent = loading ? "Loading runs..." : "No runs.";
        runListRoot.append(empty);
        return;
      }

      for (const run of runs) {
        runListRoot.append(renderRunEntry(run, state.selectedRunId, loading));
      }
    }

    function renderRunEntry(run, selectedRunId, disabled) {
      const button = document.createElement("button");
      const status = typeof run.status === "string" ? run.status : "running";
      button.type = "button";
      button.className = "run-entry " + status + (run.runId === selectedRunId ? " selected" : "");
      button.disabled = disabled;
      button.title = typeof run.runId === "string" ? run.runId : "";
      button.addEventListener("click", () => {
        if (typeof run.runId === "string" && run.runId.length > 0) {
          vscodeApi.postMessage({ type: "resumeRun", runId: run.runId });
        }
      });

      const title = document.createElement("div");
      title.className = "run-entry-title";
      title.textContent = runTitle(run);

      const meta = document.createElement("div");
      meta.className = "run-entry-meta";
      for (const part of runMeta(run)) {
        const item = document.createElement("span");
        item.textContent = part;
        meta.append(item);
      }

      button.append(title, meta);
      return button;
    }

    function runTitle(run) {
      if (typeof run.title === "string" && run.title.length > 0) {
        return run.title;
      }
      return typeof run.runId === "string" && run.runId.length > 0 ? run.runId : "Untitled run";
    }

    function runMeta(run) {
      const parts = [];
      if (typeof run.status === "string") {
        parts.push(run.status);
      }
      if (typeof run.mode === "string") {
        parts.push(run.mode);
      }
      if (typeof run.eventCount === "number") {
        parts.push(run.eventCount + " events");
      }
      const updated = formatRunTime(run.updatedAt);
      if (updated) {
        parts.push(updated);
      }
      return parts;
    }

    function formatRunTime(value) {
      if (typeof value !== "string" || value.length === 0) {
        return "";
      }
      const date = new Date(value);
      if (Number.isNaN(date.getTime())) {
        return value;
      }
      return date.toLocaleString(undefined, {
        month: "short",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      });
    }

    function renderSubmission(submission) {
      const state = submission && typeof submission === "object" ? submission : initialSubmission;
      const status = typeof state.status === "string" ? state.status : "idle";
      const busy = state.busy === true;
      setComposerBusy(busy);
      submissionRoot.className = "submission " + status;
      submissionRoot.textContent = typeof state.message === "string" ? state.message : "";
      if (status === "running") {
        promptInput.value = "";
      }
    }

    function setComposerBusy(busy) {
      promptInput.disabled = busy;
      modeInput.disabled = busy;
      sendButton.disabled = busy;
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
