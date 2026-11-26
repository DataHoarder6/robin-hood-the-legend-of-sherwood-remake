import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { JSDOM } from "jsdom";

const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");

test("index.html declares a touch-safe, safe-area-aware viewport", () => {
  const doc = JSDOM.fragment(html);
  const viewport = doc.querySelector('meta[name="viewport"]')?.getAttribute("content") ?? "";
  assert.match(viewport, /viewport-fit=cover/u);
  const style = doc.querySelector("style")?.textContent ?? "";
  assert.match(style, /#canvas\s*\{[^}]*touch-action:\s*none/u);
  assert.match(style, /env\(safe-area-inset-top(?:,\s*0px)?\)/u);
  assert.match(style, /env\(safe-area-inset-bottom(?:,\s*0px)?\)/u);
  assert.match(style, /100dvh/u);
  assert.match(style, /\.touch-controls/u);
});

test("index.html sizes the canvas to the viewport with no fixed gutter", () => {
  const doc = JSDOM.fragment(html);
  const style = doc.querySelector("style")?.textContent ?? "";
  const canvasRule = style.match(/#canvas\s*\{[^}]*\}/u)?.[0] ?? "";
  assert.ok(canvasRule, "the #canvas rule must be present");
  assert.doesNotMatch(canvasRule, /16px/u, "no hard-coded 16 px margin");
  assert.match(canvasRule, /margin:\s*0/u, "no fixed gutter around the canvas");
  assert.match(canvasRule, /100dvh/u, "height follows the dynamic viewport");
  assert.match(style, /--safe-left:\s*env\(safe-area-inset-left,\s*0px\)/u);
  const fullscreenRule = style.match(/^\s*#canvas:fullscreen\s*\{[^}]*\}/mu)?.[0] ?? "";
  assert.match(fullscreenRule, /100dvh/u);
  assert.match(fullscreenRule, /!important/u, "inline sizing must not win in fullscreen");
});
