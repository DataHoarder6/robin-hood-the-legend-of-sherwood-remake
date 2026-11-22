import assert from "node:assert/strict";
import test from "node:test";

const swUrl = new URL("../public/sw.js", import.meta.url).href;
const worker = (await import(swUrl)) as {
  isCacheable: (url: string, pinnedGeneration: string | undefined) => boolean;
  cacheNameFor: (generation: string) => string;
  isStale: (cachedGeneration: string | undefined, latestGeneration: string) => boolean;
};
const { isCacheable, cacheNameFor, isStale } = worker;

test("the service worker caches the shell, manifest, icons and only the pinned Demo generation", () => {
  assert.equal(isCacheable("/", "v18"), true);
  assert.equal(isCacheable("/manifest.webmanifest", "v18"), true);
  assert.equal(isCacheable("/icons/icon-192.png", "v18"), true);
  assert.equal(isCacheable("/datadirs/demo/v18/v18-web-opus-q80.rhdata.zst", "v18"), true);
});

test("the service worker refuses other generations, full content and foreign origins", () => {
  assert.equal(isCacheable("/datadirs/demo/v17/v17-web-opus-q80.rhdata.zst", "v18"), false);
  assert.equal(isCacheable("/datadirs/full/abc/datadir.bin", "v18"), false);
  assert.equal(isCacheable("https://other.example/x", "v18"), false);
  assert.equal(isCacheable("/datadirs/demo/v18/thing", undefined), false);
});

test("cache generations are versioned and a missing or old generation is stale", () => {
  assert.equal(cacheNameFor("v18"), "robin-shell-v18");
  assert.equal(isStale(undefined, "v18"), true);
  assert.equal(isStale("v17", "v18"), true);
  assert.equal(isStale("v18", "v18"), false);
});
