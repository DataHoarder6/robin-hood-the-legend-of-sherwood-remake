// Hermetic browser-boot e2e for issue #121: a real HTTP fixture server, real
// `fetch`, the real preload module and the real `bootGame` orchestration. Only
// the runtime and the datadir are stubbed — neither can hide a stalled asset.
// No published origin, certificate, WASM artifact or CDP seat is involved.
import assert from "node:assert/strict";
import { createServer, type Server } from "node:http";
import type { AddressInfo, Socket } from "node:net";
import test from "node:test";
import { preloadRuntimeAssets } from "./asset-preload.ts";
import { bootGame, type BootDependencies, type RobinWasmModule } from "./boot-lifecycle.ts";
import type { RobinRpc } from "./replay.ts";

// The acceptance budget is 10 s. Leave headroom for CI scheduling; a boot
// that only settles because this watchdog fired still fails the assertions.
const BOOT_DEADLINE_MS = 8_000;

type Fixture = {
  readonly assets: readonly string[];
  readonly hang?: readonly string[];
  readonly missing?: readonly string[];
};

type BootRecord = {
  readonly installed: string[];
  readonly progress: { readonly fraction: number; readonly detail: string }[];
  readonly logs: string[];
  booted: boolean;
  started: boolean;
};

async function withFixture<T>(fixture: Fixture, run: (base: string) => Promise<T>): Promise<T> {
  const sockets = new Set<Socket>();
  const server: Server = createServer((request, response) => {
    const path = new URL(request.url ?? "/", "http://127.0.0.1").pathname;
    if (path === "/build/preload-assets.json") {
      response.writeHead(200, { "content-type": "application/json" });
      response.end(JSON.stringify(fixture.assets));
      return;
    }
    if (fixture.hang?.includes(path) === true) {
      // Never respond: this is the failure the original code could not bound.
      return;
    }
    if (fixture.missing?.includes(path) === true) {
      response.writeHead(404);
      response.end("not found");
      return;
    }
    response.writeHead(200, { "content-type": "application/octet-stream" });
    response.end(new Uint8Array([1, 2, 3]));
  });
  server.on("connection", (socket) => sockets.add(socket));
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const { port } = server.address() as AddressInfo;
  try {
    return await run(`http://127.0.0.1:${port}/build`);
  } finally {
    for (const socket of sockets) socket.destroy();
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }
}

function bootAgainstFixture(
  base: string,
  record: BootRecord,
  signal: AbortSignal,
): Promise<void> {
  const wasm: RobinWasmModule = {
    default: async () => {},
    wasm_boot: () => {
      record.booted = true;
    },
    wasm_preload_asset: (path) => {
      record.installed.push(path);
    },
  };
  const rpc: RobinRpc = async <T>() => undefined as T;
  const deps: BootDependencies = {
    buildsBase: base,
    prepareJoin: async () => undefined,
    resolveBuild: async () => ({ short: "fixture", source: "latest", buildBase: base }),
    loadManifest: async () => {
      throw new Error("fixture boot has no invitation");
    },
    loadRuntime: async () => wasm,
    prepareContent: async () => {
      throw new Error("fixture boot has no invitation");
    },
    loadDefaultContent: async () => ({ datadir: new Uint8Array([1]), dataBaseUrl: base }),
    preloadLocalAssets: () => {},
    preloadShippingFiles: () => {},
    preloadAssets: (module, buildBase, latest, assetSignal) =>
      preloadRuntimeAssets(module, buildBase, latest, {
        signal: assetSignal,
        fetch,
        log: (message) => record.logs.push(message),
        progress: (fraction, detail) => record.progress.push({ fraction, detail }),
      }),
    installRpc: () => rpc,
    runtimeStarted: () => {
      record.started = true;
    },
    installReplay: async () => {},
    progress: () => {},
    log: () => {},
  };
  return bootGame(deps, signal);
}

/** Aborts boot at the budget so a hang fails the test instead of the runner. */
function bootWithBudget(base: string, record: BootRecord): Promise<void> {
  const controller = new AbortController();
  const timeout = setTimeout(
    () => controller.abort(new Error(`boot did not settle within ${BOOT_DEADLINE_MS}ms`)),
    BOOT_DEADLINE_MS,
  );
  return bootAgainstFixture(base, record, controller.signal).finally(() => clearTimeout(timeout));
}

function emptyRecord(): BootRecord {
  return { installed: [], progress: [], logs: [], booted: false, started: false };
}

test("e2e: a hung asset fails boot naming it inside the boot budget", async () => {
  await withFixture(
    { assets: ["ready.bin", "hung.bin", "after.bin"], hang: ["/build/hung.bin"] },
    async (base) => {
      const record = emptyRecord();
      let message = "";
      await assert.rejects(bootWithBudget(base, record), (cause: unknown) => {
        assert.ok(cause instanceof Error);
        message = cause.message;
        assert.match(message, /hung\.bin/u);
        return true;
      });
      assert.ok(!message.includes("did not settle"), "the budget, not the boot, must not decide the outcome");
      assert.equal(record.booted, false);
      assert.equal(record.started, false);
      assert.ok(
        !record.progress.some((entry) => entry.detail === "3 / 3"),
        "a stalled asset must never be reported as preloaded",
      );
    },
  );
});

test("e2e: a complete manifest reaches N/N and a 404 names the asset URL", async () => {
  await withFixture({ assets: ["a.bin", "b.bin", "c.bin"] }, async (base) => {
    const record = emptyRecord();
    await bootWithBudget(base, record);
    assert.deepEqual(record.installed.sort(), ["a.bin", "b.bin", "c.bin"]);
    assert.ok(record.progress.some((entry) => entry.detail === "3 / 3"));
    assert.ok(record.progress.every((entry) => entry.fraction > 0 && entry.fraction <= 1));
    assert.equal(record.booted, true);
    assert.equal(record.started, true);
  });
  await withFixture(
    { assets: ["a.bin", "missing.bin"], missing: ["/build/missing.bin"] },
    async (base) => {
      const record = emptyRecord();
      await assert.rejects(bootWithBudget(base, record), (cause: unknown) => {
        assert.ok(cause instanceof Error);
        assert.match(cause.message, /missing\.bin/u);
        return true;
      });
      assert.equal(record.booted, false);
    },
  );
});
