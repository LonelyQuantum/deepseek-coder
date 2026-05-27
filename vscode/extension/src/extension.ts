import * as vscode from "vscode";

import { registerOpenChatCommand } from "./commands";
import { RpcServerManager, readRpcServerLaunchConfig } from "./rpcServer";

export function activate(context: vscode.ExtensionContext): void {
  const rpcServer = createRpcServerManager(context);
  const openChat = registerOpenChatCommand(vscode.commands, vscode.window, rpcServer);

  context.subscriptions.push(openChat);
  if (rpcServer !== undefined) {
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
