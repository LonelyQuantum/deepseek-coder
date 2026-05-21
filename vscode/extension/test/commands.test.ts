import assert from "node:assert/strict";
import test from "node:test";

import {
  OPEN_CHAT_COMMAND,
  OPEN_CHAT_READY_MESSAGE,
  type CommandRegistry,
  type DisposableLike,
  type WindowMessenger,
  registerOpenChatCommand,
} from "../src/commands.js";

test("registerOpenChatCommand registers the public command id", () => {
  const disposable: DisposableLike = { dispose: () => undefined };
  let registeredCommand: string | undefined;

  const commands: CommandRegistry = {
    registerCommand(command) {
      registeredCommand = command;
      return disposable;
    },
  };

  const window: WindowMessenger = {
    showInformationMessage: () => undefined,
  };

  const returned = registerOpenChatCommand(commands, window);

  assert.equal(registeredCommand, OPEN_CHAT_COMMAND);
  assert.equal(returned, disposable);
});

test("open chat command reports scaffold readiness", () => {
  let callback: (() => void) | undefined;
  let message: string | undefined;

  const commands: CommandRegistry = {
    registerCommand(_command, registeredCallback) {
      callback = registeredCallback;
      return { dispose: () => undefined };
    },
  };

  const window: WindowMessenger = {
    showInformationMessage(value) {
      message = value;
    },
  };

  registerOpenChatCommand(commands, window);
  assert.ok(callback);

  callback();

  assert.equal(message, OPEN_CHAT_READY_MESSAGE);
});
