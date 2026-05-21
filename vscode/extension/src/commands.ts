export const OPEN_CHAT_COMMAND = "deepseek-coder.openChat";
export const OPEN_CHAT_READY_MESSAGE = "deepseek-coder workspace is ready.";

export interface CommandRegistry {
  registerCommand(command: string, callback: () => void): DisposableLike;
}

export interface WindowMessenger {
  showInformationMessage(message: string): unknown;
}

export interface DisposableLike {
  dispose(): unknown;
}

export function registerOpenChatCommand(
  commands: CommandRegistry,
  window: WindowMessenger,
): DisposableLike {
  return commands.registerCommand(OPEN_CHAT_COMMAND, () => {
    void window.showInformationMessage(OPEN_CHAT_READY_MESSAGE);
  });
}
