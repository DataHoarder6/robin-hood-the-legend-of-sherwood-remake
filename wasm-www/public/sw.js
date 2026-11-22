// Service worker for the browser client. It caches the shell and the Demo
// generation only. Full content is private, digest-pinned and never cached.
// The pure predicates are exported so the Node test runner can verify them.

const ORIGIN =
  typeof self !== "undefined" && self.location
    ? self.location.origin
    : "https://robin.invalid";
const ISOLATION_HEADERS = [
  "cross-origin-opener-policy",
  "cross-origin-embedder-policy",
  "cross-origin-resource-policy",
];
const SHELL_PATHS = new Set(["/", "/index.html", "/manifest.webmanifest"]);
const SHELL_PREFIXES = ["/assets/", "/icons/"];

export function cacheNameFor(generation) {
  return `robin-shell-${generation}`;
}

export function isStale(cachedGeneration, latestGeneration) {
  return cachedGeneration !== latestGeneration;
}

export function isCacheable(url, pinnedGeneration) {
  let parsed;
  try {
    parsed = new URL(url, ORIGIN);
  } catch {
    return false;
  }
  if (parsed.origin !== ORIGIN) return false;
  const path = parsed.pathname;
  if (path.startsWith("/datadirs/full/")) return false;
  if (path.startsWith("/datadirs/")) {
    return (
      pinnedGeneration !== undefined &&
      path.startsWith(`/datadirs/demo/${pinnedGeneration}/`)
    );
  }
  return SHELL_PATHS.has(path) || SHELL_PREFIXES.some((prefix) => path.startsWith(prefix));
}

function generationFromManifest(manifest) {
  const demo = manifest?.demo;
  if (typeof demo?.generation === "string") return demo.generation;
  const url = typeof demo?.url === "string" ? demo.url : undefined;
  const match = url?.match(/^\/datadirs\/demo\/([0-9a-zA-Z._-]+)\//u);
  return match?.[1];
}

async function pinnedGeneration() {
  try {
    const response = await fetch("/wasm/latest.json", { cache: "no-store" });
    if (!response.ok) return undefined;
    return generationFromManifest(await response.json());
  } catch {
    return undefined;
  }
}

const IN_SERVICE_WORKER =
  typeof self !== "undefined" && typeof self.skipWaiting === "function";

if (IN_SERVICE_WORKER) {
  self.addEventListener("install", () => {
    self.skipWaiting();
  });

  self.addEventListener("activate", (event) => {
    event.waitUntil(
      (async () => {
        const generation = await pinnedGeneration();
        const keep = generation === undefined ? null : cacheNameFor(generation);
        const names = await caches.keys();
        await Promise.all(names.filter((name) => name !== keep).map((name) => caches.delete(name)));
        await self.clients.claim();
      })(),
    );
  });

  self.addEventListener("message", (event) => {
    if (event.data?.type === "generation-changed") {
      event.waitUntil(
        (async () => {
          const names = await caches.keys();
          await Promise.all(names.map((name) => caches.delete(name)));
        })(),
      );
    }
  });

  self.addEventListener("fetch", (event) => {
    if (event.request.method !== "GET") return;
    const requestUrl = new URL(event.request.url);
    if (requestUrl.origin !== self.location.origin) return;
    event.respondWith(
      (async () => {
        try {
          const response = await fetch(event.request);
          if (
            !response.ok ||
            response.type === "opaque" ||
            !ISOLATION_HEADERS.every((header) => response.headers.has(header))
          ) {
            return response;
          }
          const generation = await pinnedGeneration();
          if (generation !== undefined && isCacheable(requestUrl.pathname, generation)) {
            const cache = await caches.open(cacheNameFor(generation));
            await cache.put(event.request, response.clone());
          }
          return response;
        } catch (error) {
          const cached = await caches.match(event.request);
          if (cached !== undefined) return cached;
          throw error;
        }
      })(),
    );
  });
}
