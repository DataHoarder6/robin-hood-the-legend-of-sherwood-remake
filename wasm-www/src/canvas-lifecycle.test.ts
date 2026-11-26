import assert from "node:assert/strict";
import test from "node:test";
import { JSDOM } from "jsdom";
import { installCanvasBackingStore } from "./canvas-lifecycle.ts";

function canvasWithMeasuredStyle(dom: JSDOM): HTMLCanvasElement {
  const canvas = dom.window.document.querySelector("canvas")!;
  canvas.getBoundingClientRect = () =>
    ({
      width: Number.parseFloat(canvas.style.width) || 0,
      height: Number.parseFloat(canvas.style.height) || 0,
    }) as DOMRect;
  return canvas;
}

test("canvas lifecycle preserves HiDPI sizing and ignores callbacks after disposal", (t) => {
  const dom = new JSDOM("<canvas></canvas>");
  t.after(() => dom.window.close());
  const canvas = dom.window.document.querySelector("canvas")!;
  canvas.getBoundingClientRect = () => ({ width: 800, height: 600 }) as DOMRect;
  let viewport = { innerWidth: 1200, innerHeight: 900, fullscreen: false, devicePixelRatio: 2 };
  let subscribed!: () => void,
    disposals = 0;
  const lifecycle = installCanvasBackingStore(canvas, {
    viewport: () => viewport,
    subscribe: (sync) => {
      subscribed = sync;
      return () => {
        disposals++;
      };
    },
  });
  assert.equal(canvas.width, 1600);
  assert.equal(canvas.height, 1200);
  assert.equal(canvas.style.width, "1200px");
  assert.equal(canvas.style.height, "900px");
  viewport = { innerWidth: 1600, innerHeight: 900, fullscreen: true, devicePixelRatio: 1 };
  subscribed();
  assert.equal(canvas.style.width, "1600px");
  assert.equal(canvas.style.height, "900px");
  assert.equal(canvas.width, 800);
  lifecycle.dispose();
  lifecycle.dispose();
  assert.equal(disposals, 1);
  viewport = { ...viewport, devicePixelRatio: 3 };
  subscribed();
  lifecycle.sync();
  assert.equal(canvas.width, 800);
});

test("the canvas fills the viewport without a fixed margin at every supported size", (t) => {
  const viewports: ReadonlyArray<readonly [number, number]> = [
    [1920, 1080],
    [1366, 768],
    [800, 480],
    [390, 844],
  ];
  for (const [innerWidth, innerHeight] of viewports) {
    const dom = new JSDOM("<canvas></canvas>");
    t.after(() => dom.window.close());
    const canvas = canvasWithMeasuredStyle(dom);
    const lifecycle = installCanvasBackingStore(canvas, {
      viewport: () => ({
        innerWidth,
        innerHeight,
        fullscreen: false,
        devicePixelRatio: 2,
      }),
      subscribe: () => () => {},
    });
    assert.equal(canvas.style.width, `${innerWidth}px`);
    assert.equal(canvas.style.height, `${innerHeight}px`);
    assert.equal(canvas.width, innerWidth * 2);
    assert.equal(canvas.height, innerHeight * 2);
    lifecycle.dispose();
  }
});

test("safe-area insets shrink the box; fullscreen ignores them and fills the viewport", (t) => {
  const dom = new JSDOM("<canvas></canvas>");
  t.after(() => dom.window.close());
  const canvas = canvasWithMeasuredStyle(dom);
  let viewport = {
    innerWidth: 390,
    innerHeight: 844,
    fullscreen: false,
    devicePixelRatio: 1,
    insets: { top: 40, right: 20, bottom: 30, left: 10 },
  };
  const lifecycle = installCanvasBackingStore(canvas, {
    viewport: () => viewport,
    subscribe: () => () => {},
  });
  assert.equal(canvas.style.width, "360px");
  assert.equal(canvas.style.height, "774px");
  viewport = { ...viewport, fullscreen: true };
  lifecycle.sync();
  assert.equal(canvas.style.width, "390px");
  assert.equal(canvas.style.height, "844px");
});

test("document fullscreen is detected even when the canvas is not the fullscreen element", (t) => {
  const dom = new JSDOM("<canvas></canvas>");
  const previous = ["window", "document", "ResizeObserver"].map(
    (key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
  );
  t.after(() => {
    for (const [key, descriptor] of previous) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else Reflect.deleteProperty(globalThis, key);
    }
    dom.window.close();
  });
  Object.defineProperty(globalThis, "window", { configurable: true, value: dom.window });
  Object.defineProperty(globalThis, "document", { configurable: true, value: dom.window.document });
  Object.defineProperty(globalThis, "ResizeObserver", {
    configurable: true,
    value: class {
      observe() {}
      disconnect() {}
    },
  });
  Object.defineProperty(dom.window, "innerWidth", { configurable: true, value: 390 });
  Object.defineProperty(dom.window, "innerHeight", { configurable: true, value: 844 });
  dom.window.document.documentElement.style.setProperty("--safe-left", "10px");
  // F11 / the fullscreen button can put the document (not the canvas) in
  // fullscreen. The old `fullscreenElement === canvas` test kept the 16 px
  // gutter in that case.
  Object.defineProperty(dom.window.document, "fullscreenElement", {
    configurable: true,
    value: dom.window.document.documentElement,
  });
  const canvas = canvasWithMeasuredStyle(dom);
  const lifecycle = installCanvasBackingStore(canvas);
  assert.equal(canvas.style.width, "390px", "document fullscreen must not shrink for insets");
  lifecycle.dispose();
});

test("browser resize delivery defers layout writes and cancels queued work on disposal", (t) => {
  const dom = new JSDOM("<canvas></canvas>");
  const previous = ["window", "document", "ResizeObserver"].map(
    (key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
  );
  t.after(() => {
    for (const [key, descriptor] of previous) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else Reflect.deleteProperty(globalThis, key);
    }
    dom.window.close();
  });
  let notify!: () => void;
  let pending: FrameRequestCallback | undefined;
  let scheduled = 0;
  Object.defineProperty(globalThis, "window", { configurable: true, value: dom.window });
  Object.defineProperty(globalThis, "document", { configurable: true, value: dom.window.document });
  Object.defineProperty(globalThis, "ResizeObserver", {
    configurable: true,
    value: class {
      constructor(callback: () => void) {
        notify = callback;
      }
      observe() {}
      disconnect() {}
    },
  });
  dom.window.requestAnimationFrame = (callback) => {
    pending = callback;
    return ++scheduled;
  };
  dom.window.cancelAnimationFrame = () => {
    pending = undefined;
  };
  const canvas = dom.window.document.querySelector("canvas")!;
  let measuredWidth = 800;
  canvas.getBoundingClientRect = () => ({ width: measuredWidth, height: 600 }) as DOMRect;
  const lifecycle = installCanvasBackingStore(canvas);
  assert.equal(canvas.width, 800);
  measuredWidth = 900;
  notify();
  notify();
  assert.equal(canvas.width, 800, "observer delivery must not synchronously change layout");
  assert.equal(scheduled, 1);
  pending!(0);
  assert.equal(canvas.width, 900);
  notify();
  lifecycle.dispose();
  assert.equal(pending, undefined);
});
