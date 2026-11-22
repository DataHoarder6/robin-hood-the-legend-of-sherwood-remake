import assert from "node:assert/strict";
import test from "node:test";
import { JSDOM } from "jsdom";
import type { RobinRpc } from "./replay.ts";
import { installTouchControls } from "./touch-controls.ts";

const MARKUP = `<div id="touch-controls" class="touch-controls" hidden>
<button data-robin-command='{"ScriptKeyPressed":{"virtual_key":27}}'></button>
<button data-robin-command='{"ScriptKeyPressed":{"virtual_key":32}}'></button>
<button data-robin-command='{"ClearAlliedSelection":null}'></button>
</div>`;

function setup() {
  const dom = new JSDOM(MARKUP);
  const root = dom.window.document;
  const calls: { method: string; params: unknown }[] = [];
  const rpc = (async (method: string, params?: unknown) => {
    calls.push({ method, params });
    return undefined;
  }) as RobinRpc;
  return { dom, root, calls, rpc };
}

test("touch controls are shown on touch devices and emit parsed player commands", () => {
  const { dom, root, calls, rpc } = setup();
  const dispose = installTouchControls(root, rpc, true);
  assert.equal(root.querySelector("#touch-controls")?.hasAttribute("hidden"), false);
  const buttons = root.querySelectorAll<HTMLButtonElement>("#touch-controls button");
  for (const button of buttons) button.click();
  assert.deepEqual(calls, [
    { method: "command", params: { ScriptKeyPressed: { virtual_key: 27 } } },
    { method: "command", params: { ScriptKeyPressed: { virtual_key: 32 } } },
    { method: "command", params: { ClearAlliedSelection: null } },
  ]);
  dispose();
  dom.window.close();
});

test("touch controls stay hidden and inert without touch", () => {
  const { dom, root, calls, rpc } = setup();
  const dispose = installTouchControls(root, rpc, false);
  assert.equal(root.querySelector("#touch-controls")?.hasAttribute("hidden"), true);
  for (const button of root.querySelectorAll<HTMLButtonElement>("#touch-controls button")) {
    button.click();
  }
  assert.deepEqual(calls, []);
  dispose();
  dom.window.close();
});

test("disposing touch controls removes listeners and a second install is a no-op", () => {
  const { dom, root, calls, rpc } = setup();
  const dispose = installTouchControls(root, rpc, true);
  assert.equal(installTouchControls(root, rpc, true), dispose);
  const button = root.querySelector<HTMLButtonElement>("#touch-controls button")!;
  button.click();
  assert.equal(calls.length, 1);
  dispose();
  button.click();
  assert.equal(calls.length, 1);
  dom.window.close();
});
