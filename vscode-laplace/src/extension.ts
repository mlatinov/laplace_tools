import * as vscode from "vscode";
import {
  HandleDiagnosticsSignature,
  LanguageClient,
  LanguageClientOptions,
  ProvideHoverSignature,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";

import { filterDiagnostics } from "./diagnosticFilter";
import { hasMathFence, renderMathFences } from "./docs";
import { clearCache, renderMath, Theme } from "./math";

let client: LanguageClient | undefined;

/// Settings a user flips often enough to want a command for, rather than a
/// trip to the settings editor.
const RENDER_MATH = "docs.renderMath";
const UNSUPPORTED_SYNTAX = "diagnostics.unsupportedSyntax";

/// Write a setting back where the user will expect to find it again: into the
/// workspace when there is one, so a library being rewritten in patch-1
/// syntax can hide its diagnostics without that leaking into every other
/// project.
async function toggle(key: string): Promise<boolean> {
  const config = vscode.workspace.getConfiguration("laplace");
  const next = !config.get<boolean>(key, true);
  const target = vscode.workspace.workspaceFolders?.length
    ? vscode.ConfigurationTarget.Workspace
    : vscode.ConfigurationTarget.Global;
  await config.update(key, next, target);
  return next;
}

/// Which foreground formulas should be rendered in. High-contrast light
/// counts as light; everything else is treated as dark, which is the safer
/// default for an unknown theme (VS Code's own default is dark).
function activeTheme(): Theme {
  const { kind } = vscode.window.activeColorTheme;
  return kind === vscode.ColorThemeKind.Light || kind === vscode.ColorThemeKind.HighContrastLight
    ? "light"
    : "dark";
}

function renderMathEnabled(): boolean {
  return vscode.workspace.getConfiguration("laplace").get<boolean>("docs.renderMath", true);
}

function showUnsupportedSyntaxDiagnostics(): boolean {
  return vscode.workspace.getConfiguration("laplace").get<boolean>(UNSUPPORTED_SYNTAX, true);
}

/// A reminder in the status bar, shown only while patch-1 diagnostics are
/// hidden. Suppressing them also suppresses every `stanc` diagnostic, so
/// leaving it on by accident means editing with Stan-level checking off --
/// worth one visible indicator, and clicking it puts them back.
function createSuppressionIndicator(): vscode.StatusBarItem {
  const item = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);
  item.command = "laplace.toggleUnsupportedSyntaxDiagnostics";
  item.text = "$(warning) Laplace: diagnostics hidden";
  item.tooltip =
    "Diagnostics from patch-1 syntax -- and everything from the stanc pass -- are hidden. Click to show them again.";
  item.backgroundColor = new vscode.ThemeColor("statusBarItem.warningBackground");
  return item;
}

/// Replace the ```` ```math ```` fences in one hover's contents with rendered
/// formulas. Anything that is not a `MarkdownString` (a plain string, or a
/// `{language, value}` pair) is left alone: neither can carry a fence.
function renderHoverMath(hover: vscode.Hover): vscode.Hover {
  const theme = activeTheme();
  let changed = false;

  const contents = hover.contents.map((part) => {
    if (typeof part === "string" || !("value" in part)) {
      return part;
    }
    if (!hasMathFence(part.value)) {
      return part;
    }
    changed = true;
    const rendered = new vscode.MarkdownString(renderMathFences(part.value, theme, renderMath));
    // A `data:image/svg+xml` image needs neither `isTrusted` (that gates
    // `command:` links) nor `supportHtml` (that gates raw HTML tags): this
    // is ordinary markdown image syntax, which is the minimum that works.
    rendered.supportThemeIcons = part instanceof vscode.MarkdownString ? part.supportThemeIcons : false;
    return rendered;
  });

  return changed ? new vscode.Hover(contents, hover.range) : hover;
}

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
    middleware: {
      // `laplace-lsp` has no `resolveCompletionItem` and no signature-help
      // provider, and its completion items carry only `@brief` -- never a
      // math section -- so the hover is the one place docs can contain a
      // formula. If that changes, the same two lines go on those providers.
      provideHover: async (document, position, token, next: ProvideHoverSignature) => {
        const hover = await next(document, position, token);
        if (!hover || !renderMathEnabled()) {
          return hover;
        }
        return renderHoverMath(hover);
      },
      handleDiagnostics: (
        uri: vscode.Uri,
        diagnostics: vscode.Diagnostic[],
        next: HandleDiagnosticsSignature,
      ) => {
        next(uri, filterDiagnostics(diagnostics, showUnsupportedSyntaxDiagnostics()));
      },
    },
  };

  client = new LanguageClient("laplace", "Laplace Language Server", serverOptions, clientOptions);

  client.start().catch((err) => {
    vscode.window.showErrorMessage(
      `laplace-lsp failed to start (looked for "${command}" on PATH -- set "laplace.serverPath" if it's installed elsewhere): ${err}`,
    );
  });

  const indicator = createSuppressionIndicator();
  const refreshIndicator = () => {
    if (showUnsupportedSyntaxDiagnostics()) {
      indicator.hide();
    } else {
      indicator.show();
    }
  };
  refreshIndicator();
  context.subscriptions.push(indicator);

  context.subscriptions.push(
    vscode.commands.registerCommand("laplace.toggleUnsupportedSyntaxDiagnostics", async () => {
      const showing = await toggle(UNSUPPORTED_SYNTAX);
      vscode.window.setStatusBarMessage(
        showing
          ? "Laplace: showing diagnostics from unsupported (patch-1) syntax"
          : "Laplace: hiding diagnostics from unsupported (patch-1) syntax, and the stanc pass",
        4000,
      );
    }),
    vscode.commands.registerCommand("laplace.toggleRenderMath", async () => {
      const on = await toggle(RENDER_MATH);
      vscode.window.setStatusBarMessage(
        on ? "Laplace: rendering //@math as formulas" : "Laplace: showing //@math as raw LaTeX",
        4000,
      );
    }),
  );

  // Formulas are cached per theme, so a switch already misses the cache and
  // re-renders in the new colour. This only stops the old theme's images
  // being kept alive for the rest of the session.
  context.subscriptions.push(vscode.window.onDidChangeActiveColorTheme(() => clearCache()));

  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration(`laplace.${RENDER_MATH}`)) {
        clearCache();
      }
      // Diagnostics are filtered as the server publishes them, so an already
      // published set has to be recomputed. Restarting is the only way to
      // make the server re-publish for every open document.
      if (event.affectsConfiguration(`laplace.${UNSUPPORTED_SYNTAX}`)) {
        refreshIndicator();
        void client?.restart();
      }
    }),
  );

  context.subscriptions.push({
    dispose: () => {
      void client?.stop();
    },
  });
}

export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}
