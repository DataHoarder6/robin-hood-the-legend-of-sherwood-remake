import assert from "node:assert/strict";
import test from "node:test";
import { DemoDatadirError, resolvePublishedDemoDatadir } from "./demo_datadir.ts";

const sha256 = "62abd8003e69bb83f6453a4998f236e8d27fc49626e517dd5932f4451f9d8dd5";
const demo = (url: string) => ({ url, sha256, byteLength: 11211392 });
const context = {
  pageOrigin: "https://robin.jautis.lt",
  binariesBase: "https://robin.jautis.lt",
  buildShort: "95ab7b9dbdf2",
};

test("a manifest on the page origin is accepted, whatever the hostname", () => {
  const result = resolvePublishedDemoDatadir(
    demo(`${context.pageOrigin}/datadirs/demo-local/datadir.bin`),
    context,
  );
  assert.equal(result.url, `${context.pageOrigin}/datadirs/demo-local/datadir.bin`);
  assert.equal(result.identity.sha256, sha256);
});

test("a jautis-origin release URL at the pinned generation is accepted", () => {
  // datadir-release.mjs now emits this exact URL when the estate publisher
  // sets ROBIN_PUBLIC_ORIGIN; the browser must accept its own site's object.
  const url = "https://robin.jautis.lt/datadirs/demo-leicester/v23-web-opus-q80.rhdata.zst";
  const result = resolvePublishedDemoDatadir(demo(url), context);
  assert.equal(result.url, url);
  assert.equal(result.identity.sha256, sha256);
});

test("a manifest on a different origin is refused as not same-origin", () => {
  assert.throws(
    () =>
      resolvePublishedDemoDatadir(
        demo("https://robinhood.phiresky.xyz/datadirs/demo/datadir.bin"),
        context,
      ),
    DemoDatadirError,
  );
});

test("a datadir outside /datadirs/ is refused", () => {
  assert.throws(
    () => resolvePublishedDemoDatadir(demo(`${context.pageOrigin}/elsewhere/datadir.bin`), context),
    DemoDatadirError,
  );
});

test("a missing or malformed identity is refused with a reason", () => {
  assert.throws(() => resolvePublishedDemoDatadir(undefined, context), DemoDatadirError);
  assert.throws(
    () =>
      resolvePublishedDemoDatadir(
        { ...demo(`${context.pageOrigin}/datadirs/x.bin`), sha256: "bad" },
        context,
      ),
    DemoDatadirError,
  );
});
