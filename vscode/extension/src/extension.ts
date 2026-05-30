import * as vscode from "vscode";

import { ApprovalEventController } from "./approvalFlow";
import { CHAT_VIEW_ID, ProleChatViewProvider } from "./chatView";
import { registerOpenChatCommand, registerOpenSettingsCommand } from "./commands";
import { createPatchDiffPreviewController } from "./diffPreview";
import { registerFimInlineCompletionProvider } from "./fimPreviewVscode";
import { RpcServerManager, readRpcServerLaunchConfig } from "./rpcServer";

export function activate(context: vscode.ExtensionContext): void {
  const workspaceRoot = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
  const rpcServer = createRpcServerManager(context);
  const chatView = new ProleChatViewProvider(context.extensionUri, rpcServer, workspaceRoot);
  const openChat = registerOpenChatCommand(vscode.commands, vscode.window, rpcServer, chatView);
  const openSettings = registerOpenSettingsCommand(
    vscode.commands,
    {
      showInformationMessage(message) {
        return vscode.window.showInformationMessage(message);
      },
      showWarningMessage(message) {
        return vscode.window.showWarningMessage(message);
      },
      openSettings(query) {
        return vscode.commands.executeCommand("workbench.action.openSettings", query);
      },
    },
    rpcServer,
  );
  const chatViewRegistration = vscode.window.registerWebviewViewProvider(CHAT_VIEW_ID, chatView, {
    webviewOptions: {
      retainContextWhenHidden: true,
    },
  });

  context.subscriptions.push(openChat, openSettings, chatView, chatViewRegistration);
  if (rpcServer !== undefined && workspaceRoot !== undefined) {
    const patchDiffPreviewController = createPatchDiffPreviewController(context, rpcServer, workspaceRoot);
    const approvalController = new ApprovalEventController(
      rpcServer,
      vscode.window,
      {
        warn(message) {
          return vscode.window.showWarningMessage(message);
        },
      },
      undefined,
      patchDiffPreviewController,
    );
    context.subscriptions.push(patchDiffPreviewController, approvalController);
    context.subscriptions.push(registerFimInlineCompletionProvider(rpcServer));
    context.subscriptions.push(rpcServer);
    if (rpcServer.autoStart) {
      void rpcServer.start().catch((error: unknown) => {
        void vscode.window.showWarningMessage(
          `prole-coder RPC server failed to start: ${errorMessage(error)}`,
        );
      });
    }
  }
}

export function deactivate(): void {
  // VS Code disposes context subscriptions, including the RPC server manager.
}

function createRpcServerManager(context: vscode.ExtensionContext): RpcServerManager | undefined {
  const workspaceRoot = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
  if (workspaceRoot === undefined) {
    return undefined;
  }

  return new RpcServerManager({
    launch: readRpcServerLaunchConfig(vscode.workspace.getConfiguration("prole-coder.rpc")),
    workspace: {
      root: workspaceRoot,
      trusted: vscode.workspace.isTrusted,
    },
    extensionVersion: extensionVersion(context),
    notifier: {
      info(message) {
        return vscode.window.showInformationMessage(message);
      },
      warn(message) {
        return vscode.window.showWarningMessage(message);
      },
    },
  });
}

function extensionVersion(context: vscode.ExtensionContext): string {
  const packageJson = context.extension.packageJSON as { version?: unknown };
  return typeof packageJson.version === "string" ? packageJson.version : "0.1.0";
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
