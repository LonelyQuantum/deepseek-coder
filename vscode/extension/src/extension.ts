import * as vscode from "vscode";

export function activate(context: vscode.ExtensionContext): void {
  const openChat = vscode.commands.registerCommand("deepseek-coder.openChat", () => {
    void vscode.window.showInformationMessage("deepseek-coder workspace is ready.");
  });

  context.subscriptions.push(openChat);
}

export function deactivate(): void {
  // No resources to release in the workspace scaffold.
}
