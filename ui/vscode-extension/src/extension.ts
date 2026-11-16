import * as vscode from "vscode";
import { createGatewayClient } from "./gateway";
import { BetterliteChatProvider } from "./provider";

export const VENDOR = "betterlite";
export const REFRESH_COMMAND = "betterlite.refreshModels";

export function activate(context: vscode.ExtensionContext): void {
  const provider = new BetterliteChatProvider(createGatewayClient());
  context.subscriptions.push(
    provider,
    vscode.lm.registerLanguageModelChatProvider(VENDOR, provider),
    vscode.commands.registerCommand(REFRESH_COMMAND, () => provider.refresh()),
  );
}

export function deactivate(): void {}
