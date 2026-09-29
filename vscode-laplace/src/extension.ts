import * as vscode from "vscode";
import {
  HandleDiagnosticsSignature,
  LanguageClient,
  LanguageClientOptions,
  ProvideHoverSignature,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";

import { DocumentContext, filterDiagnostics, firstUnsupportedLine } from "./diagnosticFilter";
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

/// What the diagnostic filter needs about the document being reported on.
///
/// The document is open -- diagnostics are only published for open documents
/// -- so this normally finds it. If it somehow cannot, every field comes back
/// empty, which makes the filter suppress nothing: failing to find the text is
/// not a reason to start hiding a user's errors.
function documentContext(uri: vscode.Uri): DocumentContext {
  const target = uri.toString();
  const document = vscode.workspace.textDocuments.find((d) => d.uri.toString() === target);
  if (!document) {
    return { firstUnsupportedLine: undefined, lineAt: () => undefined };
  }
  return {
    firstUnsupportedLine: firstUnsupportedLine(document.getText()),
    lineAt: (line) => (line < document.lineCount ? document.lineAt(line).text : undefined),
  };
}

/// A reminder in the status bar while patch-1 diagnostics are hidden.
///
/// The filter is narrow -- a missing `;` is still flagged, and so is a
/// Stan-level error above the first patch-1 construct -- but `stanc` stops at
/// its first error, so once a file opens with patch-1 syntax there is Stan
/// checking the editor cannot give. Worth being able to see that state rather
/// than forget about it.
function createSuppressionIndicator(): vscode.StatusBarItem {
  const item = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);
  item.command = "laplace.toggleUnsupportedSyntaxDiagnostics";
  item.text = "$(warning) Laplace: patch-1 diagnostics hidden";
  item.tooltip =
    "Diagnostics caused by patch-1 syntax are hidden. Real errors are still reported, " +
    "including Stan-level ones above the first patch-1 construct. Click to show everything.";
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
        next(uri, filterDiagnostics(diagnostics, showUnsupportedSyntaxDiagnostics(), documentContext(uri)));
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
          ? "Laplace: showing diagnostics caused by patch-1 syntax"
          : "Laplace: hiding diagnostics caused by patch-1 syntax -- real errors are still reported",
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
