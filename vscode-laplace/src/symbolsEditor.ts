//! The editor side of [`symbols`](./symbols): drawing the symbols over the
//! text, and offering `\sigma`-style input. Both are off unless
//! `laplace.symbols.enabled` is on, and turning it off removes every
//! decoration at once -- nothing in the file ever changes, so there is nothing
//! to undo.
//!
//! VS Code has no supported way to hide text, so the hiding is the usual
//! trick: a `textDecoration` value that smuggles `display: none` into the
//! decoration's CSS, with the symbol drawn as `before` content. If an editor
//! update ever stops honouring it, the plain names show through again; the
//! setting is the way out.

import * as vscode from "vscode";

import { codeMask, completeAt, findSymbols, SymbolSpan } from "./symbols";

export const SYMBOLS_SETTING = "symbols.enabled";

const LANGUAGES = ["laplace", "laplacelib"];

export function symbolsEnabled(): boolean {
  return vscode.workspace.getConfiguration("laplace").get<boolean>(SYMBOLS_SETTING, false);
}

function isLaplace(document: vscode.TextDocument): boolean {
  return LANGUAGES.includes(document.languageId);
}

/// Draws the symbols in every visible Laplace editor, and keeps them current.
class SymbolDisplay implements vscode.Disposable {
  private readonly plain = vscode.window.createTextEditorDecorationType({
    textDecoration: "none; display: none;",
  });
  private readonly subscripted = vscode.window.createTextEditorDecorationType({
    textDecoration: "none; display: none;",
    after: { textDecoration: "none; font-size: 0.75em; vertical-align: sub;" },
  });
  /// Spans per document version, so moving the cursor does not rescan.
  private readonly cache = new WeakMap<vscode.TextDocument, { version: number; spans: SymbolSpan[] }>();
  private pending: NodeJS.Timeout | undefined;

  refreshAll(): void {
    for (const editor of vscode.window.visibleTextEditors) {
      this.refresh(editor);
    }
  }

  /// After a keystroke, wait for a pause: rescanning on every character
  /// would make the symbols flicker while a name is half-typed anyway.
  refreshSoon(): void {
    clearTimeout(this.pending);
    this.pending = setTimeout(() => this.refreshAll(), 150);
  }

  refresh(editor: vscode.TextEditor): void {
    if (!isLaplace(editor.document)) {
      return;
    }
    if (!symbolsEnabled()) {
      editor.setDecorations(this.plain, []);
      editor.setDecorations(this.subscripted, []);
      return;
    }

    const document = editor.document;
    // The lines being edited show their real text: half a hidden name under
    // the cursor is not something anyone can edit.
    const revealed = new Set<number>();
    for (const selection of editor.selections) {
      for (let line = selection.start.line; line <= selection.end.line; line++) {
        revealed.add(line);
      }
    }

    const plain: vscode.DecorationOptions[] = [];
    const subscripted: vscode.DecorationOptions[] = [];
    for (const span of this.spansFor(document)) {
      const range = new vscode.Range(document.positionAt(span.start), document.positionAt(span.end));
      if (revealed.has(range.start.line)) {
        continue;
      }
      if (span.subscript) {
        subscripted.push({
          range,
          renderOptions: { before: { contentText: span.symbol }, after: { contentText: span.subscript } },
        });
      } else {
        plain.push({ range, renderOptions: { before: { contentText: span.symbol } } });
      }
    }
    editor.setDecorations(this.plain, plain);
    editor.setDecorations(this.subscripted, subscripted);
  }

  private spansFor(document: vscode.TextDocument): SymbolSpan[] {
    const cached = this.cache.get(document);
    if (cached && cached.version === document.version) {
      return cached.spans;
    }
    const spans = findSymbols(document.getText());
    this.cache.set(document, { version: document.version, spans });
    return spans;
  }

  dispose(): void {
    clearTimeout(this.pending);
    this.plain.dispose();
    this.subscripted.dispose();
  }
}

/// `\sigma`, `\sigma_{obs}`, `\leq` ... completed to the plain text Stan
/// accepts. Never in a comment or string: a `//@math` section is LaTeX that
/// has to stay LaTeX.
const completions: vscode.CompletionItemProvider = {
  provideCompletionItems(document, position) {
    if (!symbolsEnabled()) {
      return undefined;
    }
    const line = document.lineAt(position.line).text;
    const result = completeAt(line.slice(0, position.character), line.slice(position.character));
    if (!result) {
      return undefined;
    }
    const backslash = document.offsetAt(new vscode.Position(position.line, result.start));
    if (!codeMask(document.getText())[backslash]) {
      return undefined;
    }

    const range = new vscode.Range(position.line, result.start, position.line, result.end);
    const items = result.items.map((c, i) => {
      const item = new vscode.CompletionItem({ label: c.label, description: c.symbol }, vscode.CompletionItemKind.Text);
      item.insertText = c.insert;
      item.filterText = c.label;
      item.range = range;
      item.detail = `inserts \`${c.insert}\`, shown as ${c.symbol}`;
      item.sortText = String(i).padStart(3, "0");
      return item;
    });
    // Incomplete, so typing on into a subscript asks again.
    return new vscode.CompletionList(items, true);
  },
};

export function activateSymbols(context: vscode.ExtensionContext): void {
  const display = new SymbolDisplay();
  display.refreshAll();

  context.subscriptions.push(
    display,
    vscode.window.onDidChangeVisibleTextEditors(() => display.refreshAll()),
    vscode.window.onDidChangeTextEditorSelection((event) => display.refresh(event.textEditor)),
    vscode.workspace.onDidChangeTextDocument((event) => {
      if (isLaplace(event.document)) {
        display.refreshSoon();
      }
    }),
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration(`laplace.${SYMBOLS_SETTING}`)) {
        display.refreshAll();
      }
    }),
    vscode.languages.registerCompletionItemProvider(
      LANGUAGES.map((language) => ({ scheme: "file", language })),
      completions,
      "\\",
      "_",
      "{",
      "}",
    ),
  );
}
