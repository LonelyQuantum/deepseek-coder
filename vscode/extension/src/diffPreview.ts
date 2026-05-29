import * as path from "node:path";
import * as vscode from "vscode";

import {
  PatchDiffPreviewController,
  type PatchDiffOpenRequest,
  type PatchDiffPreviewHost,
  type PatchEventSource,
} from "./patchPreview";

export const PATCH_PREVIEW_SCHEME = "prole-patch-preview";

export function createPatchDiffPreviewController(
  context: vscode.ExtensionContext,
  eventSource: PatchEventSource,
  workspaceRoot: string,
  window: Pick<typeof vscode.window, "showWarningMessage"> = vscode.window,
): PatchDiffPreviewController {
  const provider = new PatchPreviewDocumentProvider();
  const registration = vscode.workspace.registerTextDocumentContentProvider(PATCH_PREVIEW_SCHEME, provider);
  context.subscriptions.push(provider, registration);

  return new PatchDiffPreviewController(
    eventSource,
    new VscodePatchDiffPreviewHost(workspaceRoot, provider, window),
  );
}

class VscodePatchDiffPreviewHost implements PatchDiffPreviewHost {
  constructor(
    private readonly workspaceRoot: string,
    private readonly provider: PatchPreviewDocumentProvider,
    private readonly window: Pick<typeof vscode.window, "showWarningMessage">,
  ) {}

  async readWorkspaceFile(relativePath: string): Promise<string | undefined> {
    let uri: vscode.Uri;
    try {
      uri = vscode.Uri.file(resolveWorkspacePath(this.workspaceRoot, relativePath));
    } catch (error) {
      this.warn(`prole-coder cannot preview patch path ${relativePath}: ${errorMessage(error)}`);
      return undefined;
    }

    try {
      const bytes = await vscode.workspace.fs.readFile(uri);
      return new TextDecoder("utf-8").decode(bytes);
    } catch {
      return undefined;
    }
  }

  async openDiff(request: PatchDiffOpenRequest): Promise<unknown> {
    const leftUri =
      request.file.oldPath === undefined
        ? this.virtualDocumentUri(request, "before", request.beforeContent, request.file.displayPath)
        : vscode.Uri.file(resolveWorkspacePath(this.workspaceRoot, request.file.oldPath));
    const rightUri = this.virtualDocumentUri(request, "after", request.afterContent, request.file.displayPath);
    const title = `ProleCoder: ${request.file.displayPath} (${formatHunkCount(request.file.hunks.length)})`;

    return vscode.commands.executeCommand("vscode.diff", leftUri, rightUri, title, { preview: false });
  }

  warn(message: string): unknown {
    return this.window.showWarningMessage(message);
  }

  private virtualDocumentUri(
    request: PatchDiffOpenRequest,
    side: "before" | "after",
    content: string,
    displayPath: string,
  ): vscode.Uri {
    const basename = path.basename(displayPath) || "patch";
    const uri = vscode.Uri.from({
      scheme: PATCH_PREVIEW_SCHEME,
      path: `/${encodeURIComponent(request.runId)}/${encodeURIComponent(request.approvalId)}/${request.file.fileIndex}-${side}/${encodeURIComponent(basename)}`,
      query: `toolCallId=${encodeURIComponent(request.toolCallId)}&side=${side}`,
    });
    this.provider.setContent(uri, content);
    return uri;
  }
}

class PatchPreviewDocumentProvider implements vscode.TextDocumentContentProvider, vscode.Disposable {
  private readonly documents = new Map<string, string>();
  private readonly didChange = new vscode.EventEmitter<vscode.Uri>();
  readonly onDidChange = this.didChange.event;

  setContent(uri: vscode.Uri, content: string): void {
    this.documents.set(uri.toString(), content);
    this.didChange.fire(uri);
  }

  provideTextDocumentContent(uri: vscode.Uri): string {
    return this.documents.get(uri.toString()) ?? "";
  }

  dispose(): void {
    this.documents.clear();
    this.didChange.dispose();
  }
}

function resolveWorkspacePath(workspaceRoot: string, relativePath: string): string {
  if (path.isAbsolute(relativePath)) {
    throw new Error("patch path must be workspace-relative");
  }

  const root = path.resolve(workspaceRoot);
  const resolved = path.resolve(root, relativePath);
  const rootPrefix = root.endsWith(path.sep) ? root : `${root}${path.sep}`;
  if (resolved !== root && !resolved.startsWith(rootPrefix)) {
    throw new Error("patch path escapes the workspace");
  }

  return resolved;
}

function formatHunkCount(count: number): string {
  return count === 1 ? "1 hunk" : `${count} hunks`;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
