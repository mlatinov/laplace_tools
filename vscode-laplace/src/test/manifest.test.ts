import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

const root = join(__dirname, "..", "..");
const pkg = JSON.parse(readFileSync(join(root, "package.json"), "utf8")) as {
  contributes: {
    commands?: { command: string; title: string; category?: string }[];
    configuration: { properties: Record<string, { type: string; default: unknown }> };
  };
};
const source = readFileSync(join(root, "src", "extension.ts"), "utf8");

const commands = pkg.contributes.commands ?? [];
const settings = pkg.contributes.configuration.properties;

test("every declared command has a handler", () => {
  // A command in the manifest with no `registerCommand` shows up in the
  // palette and then fails with "command not found".
  assert.ok(commands.length > 0, "no commands declared");
  for (const { command } of commands) {
    assert.ok(
      source.includes(`registerCommand("${command}"`),
      `${command} is declared but never registered`,
    );
  }
});

test("every registered command is declared", () => {
  // The reverse: a handler with no manifest entry is unreachable from the
  // palette, so the user cannot find it.
  const registered = [...source.matchAll(/registerCommand\("([^"]+)"/g)].map((m) => m[1]);
  for (const command of registered) {
    assert.ok(
      commands.some((c) => c.command === command),
      `${command} is registered but not declared in package.json`,
    );
  }
});

test("the setting the command toggles exists, and is a boolean", () => {
  for (const key of ["laplace.docs.renderMath"]) {
    const setting = settings[key];
    assert.ok(setting, `${key} is not contributed, so it cannot be changed in the settings editor`);
    assert.equal(setting.type, "boolean", `${key} should be a boolean`);
    assert.equal(setting.default, true, `${key} should default to true`);
  }
});

test("commands are grouped under one palette category", () => {
  for (const c of commands) {
    assert.equal(c.category, "Laplace", `${c.command} should be filed under "Laplace"`);
  }
});
