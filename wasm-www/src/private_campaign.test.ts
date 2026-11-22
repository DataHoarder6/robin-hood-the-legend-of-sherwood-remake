import assert from "node:assert/strict";
import test from "node:test";
import { resolvePrivateCampaignContent } from "./private_campaign.ts";

const TOKEN = "a".repeat(32);
const SHA = "b".repeat(64);
const BINDING = {
  url: `/datadirs/full/${SHA}/datadir.bin`,
  sha256: SHA,
  byteLength: 1024,
};

function fakeFetch(value: unknown) {
  const urls: string[] = [];
  const fetchJson = (async (url: string) => {
    urls.push(url);
    return value;
  }) as <T>(url: string, signal: AbortSignal) => Promise<T>;
  return { fetchJson, urls };
}

test("a valid private capability URL resolves a digest-pinned full campaign", async () => {
  const { fetchJson, urls } = fakeFetch(BINDING);
  const result = await resolvePrivateCampaignContent(
    new URL(`https://robin.jautis.lt/play/${TOKEN}/`),
    "abc1234",
    fetchJson,
    new AbortController().signal,
  );
  assert.deepEqual(urls, [`/play/${TOKEN}/campaign.json?build=abc1234`]);
  assert.deepEqual(result, {
    url: `https://robin.jautis.lt/datadirs/full/${SHA}/datadir.bin`,
    sha256: SHA,
    byteLength: 1024,
    parts: [`https://robin.jautis.lt/datadirs/full/${SHA}/datadir.bin`],
  });
});

test("the ordinary page and a malformed token never fetch private content", async () => {
  for (const path of ["/", `/play/${"a".repeat(31)}/`, "/play/not-a-token/"]) {
    const { fetchJson, urls } = fakeFetch(BINDING);
    const result = await resolvePrivateCampaignContent(
      new URL(`https://robin.jautis.lt${path}`),
      "abc1234",
      fetchJson,
      new AbortController().signal,
    );
    assert.equal(result, undefined, path);
    assert.deepEqual(urls, [], path);
  }
});

test("an invalid private binding fails loudly instead of falling back to Demo", async () => {
  const { fetchJson } = fakeFetch({ ...BINDING, url: "/datadirs/full/../demo/datadir.bin" });
  await assert.rejects(
    resolvePrivateCampaignContent(
      new URL(`https://robin.jautis.lt/play/${TOKEN}/`),
      "abc1234",
      fetchJson,
      new AbortController().signal,
    ),
    /private campaign content binding is invalid/u,
  );
});
