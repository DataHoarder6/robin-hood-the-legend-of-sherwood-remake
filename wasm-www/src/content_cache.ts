// Content-addressed browser cache for game data.
//
// The Cache API is persistent without a service worker, but it does not honour
// HTTP headers and a browser may evict an origin's cache at any time. Every
// entry is therefore keyed by the exact SHA-256 from the signed build manifest,
// verified on read, and treated as optional: a miss only costs a re-download.
// Cache names are versioned so a format change cannot read old bytes.

export const CONTENT_CACHE_NAME = "robin-content-v1";
const CACHE_PREFIX = "robin-content-";

export type CachedFile = {
  readonly url: string;
  readonly sha256: string;
  readonly byteLength: number;
};

type CacheLike = {
  match(request: Request): Promise<Response | undefined>;
  put(request: Request, response: Response): Promise<void>;
  delete(request: Request): Promise<boolean>;
};

type CacheStorageLike = {
  open(name: string): Promise<CacheLike>;
  keys?(): Promise<string[]>;
  delete?(name: string): Promise<boolean>;
};

export async function digestHex(bytes: Uint8Array): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
  return Array.from(digest, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

// Cache API keys are Requests, but this is never fetched over the network.
// A reserved invalid host makes a key collision with a real asset impossible.
function cacheKey(file: CachedFile): Request {
  return new Request(`https://robin-content.invalid/sha256/${file.sha256}`);
}

export async function readCachedContent(
  file: CachedFile,
  storage: CacheStorageLike | undefined = globalThis.caches,
): Promise<Uint8Array | undefined> {
  if (storage === undefined) return undefined;
  try {
    const cache = await storage.open(CONTENT_CACHE_NAME);
    const key = cacheKey(file);
    const response = await cache.match(key);
    if (response === undefined || !response.ok) return undefined;
    const bytes = new Uint8Array(await response.arrayBuffer()).slice();
    if (bytes.byteLength !== file.byteLength || (await digestHex(bytes)) !== file.sha256) {
      await cache.delete(key);
      return undefined;
    }
    return bytes;
  } catch {
    // Private browsing, quota, or a browser that removes CacheStorage:
    // none of these may fail a join.
    return undefined;
  }
}

export async function writeCachedContent(
  file: CachedFile,
  bytes: Uint8Array,
  storage: CacheStorageLike | undefined = globalThis.caches,
): Promise<void> {
  if (storage === undefined) return;
  if (bytes.byteLength !== file.byteLength || (await digestHex(bytes)) !== file.sha256) {
    throw new Error("refusing to cache content that does not match its manifest digest");
  }
  try {
    const cache = await storage.open(CONTENT_CACHE_NAME);
    await cache.put(
      cacheKey(file),
      new Response(new Uint8Array(bytes).slice(), {
        headers: {
          "Content-Type": "application/octet-stream",
          "X-Robin-Content-Sha256": file.sha256,
          "X-Robin-Content-Length": String(file.byteLength),
          "X-Robin-Content-URL": file.url,
        },
      }),
    );
  } catch {
    // CacheStorage is a performance hint, never the source of truth.
  }
}

export async function pruneStaleContentCaches(
  storage: CacheStorageLike | undefined = globalThis.caches,
): Promise<void> {
  if (storage === undefined || storage.keys === undefined || storage.delete === undefined) return;
  try {
    for (const name of await storage.keys()) {
      if (name.startsWith(CACHE_PREFIX) && name !== CONTENT_CACHE_NAME) {
        await storage.delete(name);
      }
    }
  } catch {
    // As above: a failed purge must not break a join.
  }
}
