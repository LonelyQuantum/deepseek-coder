import * as vscode from "vscode";

import { registerOpenChatCommand } from "./commands";

export function activate(context: vscode.ExtensionContext): void {
  const openChat = registerOpenChatCommand(vscode.commands, vscode.window);

  context.subscriptions.push(openChat);
}

export function deactivate(): void {
  // No resources to release in the workspace scaffold.
}
