import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { runTests } from "@vscode/test-electron";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const extensionDevelopmentPath = resolve(scriptDir, "..");
const repoRoot = resolve(extensionDevelopmentPath, "../..");
const extensionTestsPath = resolve(extensionDevelopmentPath, "out-test", "test", "electron", "index.js");
const workspacePath = resolve(repoRoot, "target", "vscode-test-workspace");
const settingsPath = resolve(workspacePath, ".vscode", "settings.json");

delete process.env.ELECTRON_RUN_AS_NODE;

mkdirSync(dirname(settingsPath), { recursive: true });
writeFileSync(
  settingsPath,
  `${JSON.stringify(
    {
      "prole-coder.rpc.autoStart": false,
    },
    null,
    2,
  )}\n`,
);

await runTests({
  extensionDevelopmentPath,
  extensionTestsPath,
  launchArgs: [`--folder-uri=${pathToFileURL(workspacePath).href}`, "--disable-extensions"],
});
