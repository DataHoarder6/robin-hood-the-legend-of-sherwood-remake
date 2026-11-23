import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import test from "node:test";
import { promisify } from "node:util";
import { CURRENT_DEMO_URL, parseDatadirRelease, readDatadirRelease } from "./datadir-release.mjs";
import { DEMO_PATH } from "./verify-datadir-corpus.mjs";

const valid = Object.freeze({
  schema: 1,
  url: CURRENT_DEMO_URL,
  sha256: "a".repeat(64),
  byte_length: 12,
  native_content_sha256: "b".repeat(64),
});

test("datadir release is a closed document naming the current Demo generation", () => {
  assert.deepEqual(parseDatadirRelease(valid), valid);
  for (const override of [
    { schema: 2 },
    { url: "https://robinhood.phiresky.xyz/datadirs/demo-leicester/v8-web-opus-q80.rhdata.zst" },
    { sha256: "A".repeat(64) },
    { native_content_sha256: "" },
    { byte_length: 0 },
    { byte_length: 25 * 1024 * 1024 + 1 },
    { extra: true },
  ]) {
    assert.throws(() => parseDatadirRelease({ ...valid, ...override }));
  }
  const { sha256: _omitted, ...missing } = valid;
  assert.throws(() => parseDatadirRelease(missing), /unexpected keys/u);
});

test("the current CLI mode writes a readable release file and never overwrites one", async (t) => {
  const root = await mkdtemp(resolve(tmpdir(), "datadir-release-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const output = resolve(root, "datadir-release.json");
  const script = resolve(import.meta.dirname, "datadir-release.mjs");
  await promisify(execFile)(process.execPath, [
    script,
    "current",
    valid.sha256,
    "12",
    valid.native_content_sha256,
    output,
  ]);
  assert.deepEqual(await readDatadirRelease(output), valid);
  await assert.rejects(
    promisify(execFile)(process.execPath, [
      script,
      "current",
      valid.sha256,
      "12",
      valid.native_content_sha256,
      output,
    ]),
    /EEXIST/u,
  );
  await writeFile(resolve(root, "broken.json"), "{");
  await assert.rejects(readDatadirRelease(resolve(root, "broken.json")), /cannot read/u);
  assert.match(await readFile(output, "utf8"), /\n$/u);
});

test("an estate publisher binds the same-origin Demo URL it serves", async (t) => {
  const origin = "https://robin.jautis.lt";
  const jautisUrl = `${origin}/${DEMO_PATH}`;
  const jautisRelease = Object.freeze({ ...valid, url: jautisUrl });
  const cloudflareUrl = `https://robinhood.phiresky.xyz/${DEMO_PATH}`;

  // The pure predicate accepts the jautis URL only when that is the origin it
  // is asked to enforce; upstream's default still rejects it.
  assert.deepEqual(parseDatadirRelease(jautisRelease, "jautis", jautisUrl), jautisRelease);
  assert.throws(
    () => parseDatadirRelease(jautisRelease, "cloudflare", cloudflareUrl),
    /Demo generation/u,
  );

  const root = await mkdtemp(resolve(tmpdir(), "datadir-release-origin-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const output = resolve(root, "datadir-release.json");
  const script = resolve(import.meta.dirname, "datadir-release.mjs");
  await promisify(execFile)(
    process.execPath,
    [script, "current", valid.sha256, "12", valid.native_content_sha256, output],
    { env: { ...process.env, ROBIN_PUBLIC_ORIGIN: origin } },
  );
  const written = JSON.parse(await readFile(output, "utf8"));
  assert.equal(written.url, jautisUrl);
  assert.deepEqual(parseDatadirRelease(written, output, jautisUrl), written);
});
