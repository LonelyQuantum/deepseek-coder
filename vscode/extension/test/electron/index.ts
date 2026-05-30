import assert from "node:assert/strict";

import * as vscode from "vscode";

const extensionId = "prole-coder.prole-coder-vscode";

export async function run(): Promise<void> {
  const extension = vscode.extensions.getExtension(extensionId);

  assert.ok(extension, `${extensionId} should be installed in the test host`);
  await extension.activate();
  assert.equal(extension.isActive, true);
  assert.equal(vscode.workspace.isTrusted, true);
  assert.equal(vscode.workspace.getConfiguration("prole-coder.rpc").get("autoStart"), false);

  await vscode.commands.executeCommand("workbench.view.extension.prole-coder");
  await vscode.commands.executeCommand("prole-coder.chat.focus");

  const commands = await vscode.commands.getCommands(true);
  assert.equal(commands.includes("prole-coder.openChat"), true);
}
