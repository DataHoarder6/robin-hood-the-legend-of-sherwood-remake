import type { RobinWasmModule } from "./boot-lifecycle.js";
import { withAbort } from "./cancellation.ts";

const PRELOAD_FETCH_CONCURRENCY = 12;
// A single stalled request must not hold the whole boot at 0/N forever. The
// budget is per asset, not per preload: twelve workers may be in flight and
// each one gets its own deadline from the moment its worker starts it.
const PRELOAD_ASSET_TIMEOUT_MS = 3_000;
type PreloadEntry = string | { readonly path?: unknown; readonly url?: unknown };
export type PreloadDependencies = {
  readonly signal?: AbortSignal;
  fetch: typeof fetch;
  log: (message: string) => void;
  progress: (fraction: number, detail: string) => void;
  /** Override only for tests; production uses `PRELOAD_ASSET_TIMEOUT_MS`. */
  readonly assetTimeoutMs?: number;
};

export function resolvePreloadEntries(
  raw: unknown,
  manifestUrl: string,
  buildBase: string,
): { path: string; assetUrl: string }[] {
  if (!Array.isArray(raw)) {
    throw new Error(`${manifestUrl} must be a JSON array`);
  }
  return (raw as PreloadEntry[]).map((entry, index) => {
    if (
      typeof entry !== "string" &&
      (entry === null || typeof entry !== "object" || Array.isArray(entry))
    ) {
      throw new Error(`${manifestUrl} contains an invalid preload entry at index ${index}`);
    }
    const path = typeof entry === "string" ? entry : entry.path;
    const url = typeof entry === "string" ? `${buildBase}/${entry}` : (entry.url ?? path);
    if (
      typeof path !== "string" ||
      typeof url !== "string" ||
      path.length === 0 ||
      url.length === 0
    ) {
      throw new Error(`${manifestUrl} contains an invalid preload entry at index ${index}`);
    }
    const assetUrl = new URL(url, buildBase.endsWith("/") ? buildBase : `${buildBase}/`).toString();
    return { path, assetUrl };
  });
}

async function readPreloadResource<T>(
  signal: AbortSignal,
  url: string,
  read: () => Promise<T>,
): Promise<T> {
  try {
    return await withAbort(signal, read);
  } catch (cause) {
    signal.throwIfAborted();
    throw new Error(`preload ${url}: ${cause instanceof Error ? cause.message : String(cause)}`, {
      cause,
    });
  }
}

/** One asset's abort set: either the caller/sibling failure or its own deadline. */
function assetFetchSignal(
  parent: AbortSignal,
  timeoutMs: number,
): { readonly signal: AbortSignal; readonly timeout: AbortSignal } {
  const timeout = AbortSignal.timeout(timeoutMs);
  return { signal: AbortSignal.any([parent, timeout]), timeout };
}

async function readAssetResource<T>(
  parent: AbortSignal,
  timed: AbortSignal,
  timeout: AbortSignal,
  url: string,
  timeoutMs: number,
  read: () => Promise<T>,
): Promise<T> {
  try {
    return await withAbort(timed, read);
  } catch (cause) {
    // A sibling's failure is the authoritative error: it names the asset
    // that failed first. Only otherwise report this asset's own timeout.
    parent.throwIfAborted();
    if (timeout.aborted) {
      throw new Error(`preload ${url}: timed out after ${timeoutMs}ms`, { cause });
    }
    throw new Error(`preload ${url}: ${cause instanceof Error ? cause.message : String(cause)}`, {
      cause,
    });
  }
}

export async function preloadRuntimeAssets(
  wasm: RobinWasmModule,
  buildBase: string,
  noCache: boolean,
  deps: PreloadDependencies,
): Promise<void> {
  const signal = deps.signal ?? new AbortController().signal;
  signal.throwIfAborted();
  if (wasm.wasm_preload_asset === undefined) {
    return;
  }
  // The first asset failure aborts this controller so the other workers stop
  // dequeuing and their in-flight fetches are cancelled. An external abort
  // keeps its own reason: `AbortSignal.any` retains whichever aborts first.
  const failed = new AbortController();
  const runSignal = AbortSignal.any([signal, failed.signal]);
  const preloadAsset = wasm.wasm_preload_asset;
  // Browser fetch rejects a dependency object as its receiver. Invoke the
  // injected function detached, as the normal global fetch call would be.
  const fetchAsset = deps.fetch;
  const manifestUrl = `${buildBase}/preload-assets.json`;
  const manifestResp = await readPreloadResource(runSignal, manifestUrl, () =>
    fetchAsset(manifestUrl, {
      cache: noCache ? "no-cache" : "force-cache",
      signal: runSignal,
    }),
  );
  if (manifestResp.status === 404) {
    return;
  }
  if (!manifestResp.ok) {
    throw new Error(`fetch ${manifestUrl}: HTTP ${manifestResp.status}`);
  }
  const raw = (await readPreloadResource(runSignal, manifestUrl, () =>
    manifestResp.json(),
  )) as unknown;
  const entries = resolvePreloadEntries(raw, manifestUrl, buildBase);
  const timeoutMs = deps.assetTimeoutMs ?? PRELOAD_ASSET_TIMEOUT_MS;

  // Fetch and install in the same bounded worker. Keeping every completed
  // ArrayBuffer until all requests finish doubles the preload peak: JS owns
  // all downloads while wasm_preload_asset copies them into Rust.
  let preloaded = 0;
  await forEachConcurrent(
    entries,
    PRELOAD_FETCH_CONCURRENCY,
    async ({ path, assetUrl }) => {
      try {
        const fetchState = assetFetchSignal(runSignal, timeoutMs);
        const assetResp = await readAssetResource(
          runSignal,
          fetchState.signal,
          fetchState.timeout,
          assetUrl,
          timeoutMs,
          () =>
            fetchAsset(assetUrl, {
              cache: noCache ? "no-cache" : "force-cache",
              signal: fetchState.signal,
            }),
        );
        if (!assetResp.ok) {
          throw new Error(`fetch ${assetUrl}: HTTP ${assetResp.status}`);
        }
        const bodyState = assetFetchSignal(runSignal, timeoutMs);
        const bytes = new Uint8Array(
          await readAssetResource(
            runSignal,
            bodyState.signal,
            bodyState.timeout,
            assetUrl,
            timeoutMs,
            () => assetResp.arrayBuffer(),
          ),
        );
        runSignal.throwIfAborted();
        preloadAsset(path, bytes);
        preloaded += 1;
        deps.progress(preloaded / entries.length, `${preloaded} / ${entries.length}`);
        deps.log(`[preloaded ${path}: ${bytes.byteLength} bytes]`);
      } catch (error) {
        // `forEachConcurrent` deliberately waits for every worker; aborting
        // here is what lets the first failure surface before its peers finish.
        failed.abort(error);
        throw error;
      }
    },
    runSignal,
  );
}

export async function forEachConcurrent<T>(
  items: readonly T[],
  concurrency: number,
  action: (item: T, index: number) => Promise<void>,
  signal?: AbortSignal,
): Promise<void> {
  if (!Number.isInteger(concurrency) || concurrency < 1) {
    throw new Error(`preload concurrency must be a positive integer, got ${concurrency}`);
  }
  let firstError: { index: number; error: Error } | undefined;
  let next = 0;
  const worker = async (): Promise<void> => {
    for (;;) {
      if (signal?.aborted) return;
      const index = next++;
      if (index >= items.length) {
        return;
      }
      try {
        await action(items[index] as T, index);
      } catch (error) {
        const normalized = error instanceof Error ? error : new Error(String(error));
        // Completion order may differ from manifest order.
        if (firstError === undefined || index < firstError.index) {
          firstError = { index, error: normalized };
        }
      }
    }
  };
  const workerCount = Math.min(concurrency, items.length);
  await Promise.all(Array.from({ length: workerCount }, worker));
  signal?.throwIfAborted();
  if (firstError !== undefined) {
    throw firstError.error;
  }
}
