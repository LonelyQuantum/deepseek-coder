import { randomUUID } from "node:crypto";
import * as vscode from "vscode";

import { ChatEventTimeline, type ChatTimelineSnapshot } from "./chatEvents";
import type { AgentEventEnvelope, DisposableLike } from "./rpcServer";

export const CHAT_VIEW_ID = "prole-coder.chat";

export interface ChatRpcEventSource {
  onEvent(handler: (event: AgentEventEnvelope) => void): DisposableLike;
}

interface WebviewMessage {
  readonly type: "snapshot";
  readonly snapshot: ChatTimelineSnapshot;
}

export class ProleChatViewProvider implements vscode.WebviewViewProvider, DisposableLike {
  private readonly timeline = new ChatEventTimeline();
  private readonly rpcEvents: ChatRpcEventSource | undefined;
  private rpcSubscription: DisposableLike | undefined;
  private view: vscode.WebviewView | undefined;

  constructor(
    private readonly extensionUri: vscode.Uri,
    rpcEvents?: ChatRpcEventSource,
  ) {
    this.rpcEvents = rpcEvents;
    this.rpcSubscription = rpcEvents?.onEvent((event) => {
      this.timeline.append(event);
      this.postSnapshot();
    });
  }

  resolveWebviewView(webviewView: vscode.WebviewView): void {
    this.view = webviewView;
    webviewView.webview.options = {
      enableScripts: true,
      localResourceRoots: [this.extensionUri],
    };
    webviewView.webview.html = renderChatViewHtml(webviewView.webview, this.timeline.snapshot());
    this.postSnapshot();
  }

  openChatView(): Thenable<unknown> {
    return vscode.commands.executeCommand(`${CHAT_VIEW_ID}.focus`);
  }

  dispose(): void {
    this.rpcSubscription?.dispose();
    this.rpcSubscription = undefined;
  }

  private postSnapshot(): void {
    const message: WebviewMessage = {
      type: "snapshot",
      snapshot: this.timeline.snapshot(),
    };
    void this.view?.webview.postMessage(message);
  }
}

function renderChatViewHtml(webview: vscode.Webview, snapshot: ChatTimelineSnapshot): string {
  const nonce = nonceValue();
  const initialSnapshot = safeScriptJson(snapshot);
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
