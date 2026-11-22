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
  assert.match(style, /env\(safe-area-inset-top\)/u);
  assert.match(style, /env\(safe-area-inset-bottom\)/u);
  assert.match(style, /100dvh/u);
  assert.match(style, /\.touch-controls/u);
});
