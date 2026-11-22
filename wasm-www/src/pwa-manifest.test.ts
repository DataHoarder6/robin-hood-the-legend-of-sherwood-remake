import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import test from "node:test";
import { JSDOM } from "jsdom";
import { cspDirectives } from "./csp-test-utils.ts";

const here: URL = new URL("../", import.meta.url);
const html = readFileSync(new URL("index.html", here), "utf8");
const headers = readFileSync(new URL("deploy/public-headers.txt", here), "utf8");

test("game page links a manifest and its CSP allows the manifest", () => {
  const doc = JSDOM.fragment(html);
  const manifest = doc.querySelector('link[rel="manifest"]');
  assert.equal(manifest?.getAttribute("href"), "/manifest.webmanifest");
  assert.ok(cspDirectives(html).get("manifest-src")?.has("'self'"));
});

test("web manifest declares an installable standalone app with real icons", () => {
  const manifest = JSON.parse(
    readFileSync(new URL("public/manifest.webmanifest", here), "utf8"),
  ) as Record<string, unknown>;
  for (const key of ["name", "short_name", "background_color", "theme_color"]) {
    assert.equal(typeof manifest[key], "string", `${key} must be a string`);
    assert.ok((manifest[key] as string).length > 0, `${key} must not be empty`);
  }
  assert.equal(manifest.display, "standalone");
  assert.equal(manifest.start_url, "/");
  const icons = manifest.icons as readonly Record<string, unknown>[];
  assert.ok(Array.isArray(icons));
  for (const size of ["192x192", "512x512"]) {
    const icon: Record<string, unknown> | undefined = icons.find((entry) => entry.sizes === size);
    assert.ok(icon, `missing ${size} icon`);
    assert.equal(icon.type, "image/png");
    const path: string = String(icon.src).replace(/^\//u, "public/");
    assert.ok(existsSync(new URL(path, here)), `missing icon file ${path}`);
  }
});

test("deploy headers allow the manifest and keep cross-origin isolation", () => {
  const csp = headers.split("\n").find((line) => line.trimStart().startsWith("Content-Security-Policy:"));
  assert.ok(csp, "public headers must declare a CSP");
  assert.match(csp, /manifest-src 'self'/u);
  assert.match(csp, /worker-src 'self' blob:/u);
  for (const header of [
    "Cross-Origin-Opener-Policy: same-origin",
    "Cross-Origin-Embedder-Policy: require-corp",
    "Cross-Origin-Resource-Policy: same-origin",
  ]) {
    assert.ok(headers.includes(header), `missing ${header}`);
  }
});
