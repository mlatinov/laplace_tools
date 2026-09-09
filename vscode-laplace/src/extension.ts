import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

export function activate(context: vscode.ExtensionContext): void {
  const config = vscode.workspace.getConfiguration("laplace");
  const command = config.get<string>("serverPath", "laplace-lsp");

  const serverOptions: ServerOptions = {
    command,
    transport: TransportKind.stdio,
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [
      { scheme: "file", language: "laplace" },
      { scheme: "file", language: "laplacelib" },
    ],
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher("**/laplace.{toml,lock}"),
    },
  };

  client = new LanguageClient("laplace", "Laplace Language Server", serverOptions, clientOptions);

  client.start().catch((err) => {
    vscode.window.showErrorMessage(
      `laplace-lsp failed to start (looked for "${command}" on PATH -- set "laplace.serverPath" if it's installed elsewhere): ${err}`,
    );
  });

  context.subscriptions.push({
    dispose: () => {
      void client?.stop();
    },
  });
}

export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}
