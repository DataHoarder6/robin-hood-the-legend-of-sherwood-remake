import assert from "node:assert/strict";
import test from "node:test";
import {
  CONTENT_CACHE_NAME,
  digestHex,
  pruneStaleContentCaches,
  readCachedContent,
  writeCachedContent,
  type CachedFile,
} from "./content_cache.ts";

function fakeStorage(entries = new Map<string, Response>()) {
  const cache = {
    async match(request: Request) {
      return entries.get(request.url);
    },
    async put(request: Request, response: Response) {
      entries.set(request.url, response.clone());
    },
    async delete(request: Request) {
      return entries.delete(request.url);
    },
  };
  return {
    entries,
    storage: {
      async open(name: string) {
        assert.equal(name, CONTENT_CACHE_NAME);
        return cache;
      },
      async keys() {
        return [CONTENT_CACHE_NAME, "robin-content-v0", "unrelated"];
      },
      async delete(name: string) {
        deleted.push(name);
        return true;
      },
    },
  };
}

const deleted: string[] = [];

async function file(bytes: Uint8Array): Promise<[CachedFile, Uint8Array]> {
  const copy = new Uint8Array(bytes);
  return [
    {
      url: "https://robin.jautis.lt/datadirs/demo/datadir.bin",
      sha256: await digestHex(copy),
      byteLength: copy.byteLength,
    },
    copy,
  ];
}

test("an exact cached file is reused without the network", async () => {
  const entries = new Map<string, Response>();
  const [descriptor, bytes] = await file(new Uint8Array([1, 2, 3]));
  const { storage } = fakeStorage(entries);
  await writeCachedContent(descriptor, bytes, storage);
  const read = await readCachedContent(descriptor, storage);
  assert.deepEqual(read, bytes);
});

test("a corrupted cache entry is deleted and reported as a miss", async () => {
  const [descriptor, bytes] = await file(new Uint8Array([1, 2, 3]));
  const entries = new Map<string, Response>([
    [
      "https://robin-content.invalid/sha256/" + descriptor.sha256,
      new Response(new Uint8Array([9, 9, 9])),
    ],
  ]);
  const { storage } = fakeStorage(entries);
  assert.equal(await readCachedContent(descriptor, storage), undefined);
  assert.equal(entries.size, 0);
  assert.equal(bytes.byteLength, 3);
});

test("missing CacheStorage is an accepted miss", async () => {
  const [descriptor] = await file(new Uint8Array([1]));
  assert.equal(await readCachedContent(descriptor, undefined), undefined);
  await writeCachedContent(descriptor, new Uint8Array([1]), undefined);
});

test("refuses to cache bytes that disagree with the manifest", async () => {
  const [descriptor] = await file(new Uint8Array([1, 2, 3]));
  const { storage } = fakeStorage();
  await assert.rejects(
    writeCachedContent(descriptor, new Uint8Array([9]), storage),
    /does not match its manifest digest/,
  );
});

test("old cache generations are pruned without touching unrelated caches", async () => {
  const { storage } = fakeStorage();
  deleted.length = 0;
  await pruneStaleContentCaches(storage);
  assert.deepEqual(deleted, ["robin-content-v0"]);
});
