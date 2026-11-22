import assert from "node:assert/strict";
import test from "node:test";
import { resolvePrivateCampaignContent } from "./private_campaign.ts";

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

test("the site root resolves the same digest-pinned generation", async () => {
  const { fetchJson, urls } = fakeFetch(BINDING);
  const result = await resolvePrivateCampaignContent(
    new URL("https://robin.jautis.lt/"),
    "abc1234",
    fetchJson,
    new AbortController().signal,
  );
  assert.deepEqual(urls, [`/campaign.json?build=abc1234`]);
  assert.equal(result?.sha256, SHA);
  assert.equal(result?.byteLength, 1024);
});

test("any path other than the origin root never fetches private content", async () => {
  for (const path of ["/demo", "/play/not-a-token/", "/leaderboards/"]) {
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
