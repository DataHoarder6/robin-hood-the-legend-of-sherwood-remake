// Batch bake of all 24 directions for one saved map.
//
// The WebGL bake in app/src/map-bake-render.ts only runs in a browser. This
// driver reuses the repo's headless-Chrome fixture (app/tests/map-bake-library.html
// via app/tests/run-lifecycle.mjs) against a staging library of cloned scene
// documents, one per yaw. It never reimplements projection: every PNG comes
// from the existing bake pipeline.
//
//   node pipeline/direction-ring-bake.ts <source.rhlos-map.json> <out-dir> [options]
//
// Options: --library <dir> --server-url <url> --chrome <path>
//          --port <n> --timeout <ms> --keep-zips
//
// Run under a shell where `pnpm` is on PATH, e.g.
//   nix shell nixpkgs#pnpm --command env npm_config_manage_package_manager_versions=false \
//     node pipeline/direction-ring-bake.ts ...

import { spawn } from "node:child_process";
import { access, mkdir, readFile, readdir, rename, rm, symlink, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import type { ChildProcess } from "node:child_process";
import { unzipSync } from "fflate";
import {
  DIRECTION_COUNT,
  directionPngName,
  directionSceneName,
  directionSteps,
  directionVariants,
  mapSlug,
} from "./direction-ring.ts";

const editorRoot = path.resolve(fileURLToPath(new URL("..", import.meta.url)));
const PNG_SUFFIX = ".map.png";
const STARTUP_TIMEOUT_MS = 90_000;
const SERVER_POLL_MS = 500;

interface BakeOptions {
  source: string;
  out: string;
  library?: string;
  serverUrl: string;
  startServer: boolean;
  chrome: string;
  port: number;
  timeout: number;
  keepZips: boolean;
}

function usage(): string {
  return (
    "usage: direction-ring-bake <source.rhlos-map.json> <out-dir> " +
    "[--library <dir>] [--server-url <url>] [--chrome <path>] " +
    "[--port <n>] [--timeout <ms>] [--keep-zips]"
  );
}

function parseOptions(argv: string[]): BakeOptions {
  const positional: string[] = [];
  const values = new Map<string, string>();
  const flags = new Set<string>();
  const takesValue = new Set(["--library", "--server-url", "--chrome", "--port", "--timeout"]);
  for (let index = 0; index < argv.length; index++) {
    const arg = argv[index];
    if (arg === undefined) continue;
    if (takesValue.has(arg)) {
      const value = argv[++index];
      if (!value || value.startsWith("--")) throw new Error(`${arg} needs a value`);
      values.set(arg, value);
    } else if (arg === "--keep-zips") flags.add(arg);
    else if (arg.startsWith("--")) throw new Error(`Unknown option ${arg}`);
    else positional.push(arg);
  }
  const [source, out] = positional;
  if (source === undefined || out === undefined) throw new Error(usage());
  const serverUrl = values.get("--server-url");
  const port = Number(
    values.get("--port") ?? (new URL(serverUrl ?? "http://127.0.0.1:5183").port || "5183"),
  );
  if (!Number.isSafeInteger(port) || port <= 0 || port > 65535)
    throw new Error(`Invalid --port: ${port}`);
  const timeout = Number(values.get("--timeout") ?? "3600000");
  if (!Number.isSafeInteger(timeout) || timeout <= 0)
    throw new Error(`Invalid --timeout: ${timeout}`);
  return {
    source,
    out,
    library: values.get("--library"),
    serverUrl: serverUrl ?? `http://127.0.0.1:${port}`,
    startServer: serverUrl === undefined,
    chrome: values.get("--chrome") ?? process.env.CHROME ?? "google-chrome",
    port,
    timeout,
    keepZips: flags.has("--keep-zips"),
  };
}

function sourceLibrary(source: string, override?: string): string {
  const scenes = path.dirname(path.resolve(source));
  if (path.basename(scenes) !== "scenes")
    throw new Error(`Source map must live in a library's scenes/ directory: ${source}`);
  const library = override ? path.resolve(override) : path.dirname(scenes);
  return library;
}

async function stageLibrary(library: string, staging: string, source: unknown): Promise<string[]> {
  const sourceMap = (source as { map: string }).map;
  const variants = directionVariants(source);
  await rm(staging, { recursive: true, force: true });
  await mkdir(path.join(staging, "scenes"), { recursive: true });
  for (const [step, document] of variants.entries()) {
    const name = directionSceneName(sourceMap, step);
    await writeFile(
      path.join(staging, "scenes", `${name}.rhlos-map.json`),
      JSON.stringify(document, null, 2) + "\n",
    );
  }
  const entries = await readdir(library, { withFileTypes: true });
  for (const entry of entries) {
    if (entry.name === "scenes") continue;
    await symlink(
      path.join(library, entry.name),
      path.join(staging, entry.name),
      entry.isDirectory() ? "dir" : "file",
    );
  }
  if (!entries.some((entry) => entry.name === "3d-assets"))
    throw new Error(
      `Library ${library} has no 3d-assets/: the WebGL bake needs its pinned GLB models; no PNG was produced`,
    );
  try {
    await access(path.join(library, "3d-assets", "index.json"));
  } catch {
    throw new Error(
      `Library ${library} has no 3d-assets/index.json: the editor cannot open a published asset catalog; no PNG was produced`,
    );
  }
  return variants.map((document) => document.map);
}

async function waitForServer(url: string, child: ChildProcess): Promise<void> {
  const deadline = Date.now() + STARTUP_TIMEOUT_MS;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) throw new Error(`Vite exited before serving ${url}`);
    try {
      const response = await fetch(`${url}/tests/map-bake-library.html`, {
        signal: AbortSignal.timeout(2000),
      });
      if (response.ok) return;
    } catch {
      // Server not listening yet.
    }
    await new Promise((resolve) => setTimeout(resolve, SERVER_POLL_MS));
  }
  throw new Error(`Vite did not serve ${url} within ${STARTUP_TIMEOUT_MS}ms`);
}

async function startServer(staging: string, port: number, url: string): Promise<ChildProcess> {
  const child = spawn(
    "pnpm",
    ["--filter", "app", "dev", "--host", "127.0.0.1", "--port", String(port), "--strictPort"],
    {
      cwd: editorRoot,
      env: {
        ...process.env,
        EDITOR_LIBRARY: staging,
        npm_config_manage_package_manager_versions:
          process.env.npm_config_manage_package_manager_versions ?? "false",
      },
      stdio: ["ignore", "pipe", "pipe"],
      detached: true,
    },
  );
  const prefix = (chunk: Buffer) => process.stdout.write(`[vite] ${String(chunk)}`);
  child.stdout?.on("data", prefix);
  child.stderr?.on("data", prefix);
  child.on("error", (error) => process.stderr.write(`[vite] ${error.message}\n`));
  await waitForServer(url, child);
  return child;
}

async function stopServer(child: ChildProcess | undefined): Promise<void> {
  if (!child || child.pid === undefined || child.exitCode !== null) return;
  try {
    process.kill(-child.pid, "SIGTERM");
  } catch {
    child.kill("SIGTERM");
  }
}

async function bakeOne(
  options: BakeOptions,
  scene: string,
  zipPath: string,
): Promise<{ stage: string; log: string }> {
  const runner = path.join(editorRoot, "app/tests/run-lifecycle.mjs");
  const env = {
    ...process.env,
    CHROME: options.chrome,
    TEST_PAGE: "map-bake-library.html",
    TEST_QUERY: `?map=${encodeURIComponent(scene)}&map-only=1`,
    TEST_BAKE_ZIP: zipPath,
    TEST_TIMEOUT: String(options.timeout),
  };
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [runner, options.serverUrl], {
      cwd: editorRoot,
      env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let log = "";
    let stage = "";
    const collect = (chunk: Buffer) => {
      const text = String(chunk);
      log = (log + text).slice(-16_384);
      for (const line of text.split("\n")) {
        if (!line) continue;
        stage = line;
        process.stdout.write(`[${scene}] ${line}\n`);
      }
    };
    child.stdout?.on("data", collect);
    child.stderr?.on("data", collect);
    child.on("error", (error) => reject(new Error(`Bake runner failed: ${error.message}`)));
    child.on("close", (code) => {
      if (code === 0) resolve({ stage, log });
      else reject(new Error(`Bake runner exited ${code} for ${scene}:\n${stage}\n${log}`));
    });
  });
}

async function extractPng(zipPath: string, compiledName: string): Promise<Uint8Array> {
  const entries = unzipSync(new Uint8Array(await readFile(zipPath)));
  const expected = `Data/Levels/Day/${compiledName}${PNG_SUFFIX}`;
  const bytes = entries[expected];
  if (!bytes) {
    const alternatives = Object.keys(entries).filter((entry) =>
      entry.endsWith(`${compiledName}${PNG_SUFFIX}`),
    );
    const alternative = alternatives.length === 1 ? alternatives[0] : undefined;
    if (alternative !== undefined) {
      const bytes = entries[alternative];
      if (bytes) return bytes;
    }
    throw new Error(`Bake ZIP omitted ${expected}; found ${alternatives.join(", ") || "nothing"}`);
  }
  return bytes;
}

export async function bakeCommand(argv: string[]): Promise<void> {
  const options = parseOptions(argv);
  const source = path.resolve(options.source);
  const out = path.resolve(options.out);
  const sourceText = await readFile(source, "utf8");
  let sourceDocument: unknown;
  try {
    sourceDocument = JSON.parse(sourceText);
  } catch (error) {
    throw new Error(`Invalid map JSON ${source}: ${String(error)}`, { cause: error });
  }
  const sourceMap = (sourceDocument as { map?: string }).map;
  if (typeof sourceMap !== "string" || !sourceMap)
    throw new Error(`Map document ${source} has no map name`);
  const library = sourceLibrary(source, options.library);
  await mkdir(out, { recursive: true });
  const staging = path.join(out, ".staging-library");
  const partial = path.join(out, ".partial");
  await rm(partial, { recursive: true, force: true });
  await mkdir(partial, { recursive: true });
  const scenes = await stageLibrary(library, staging, sourceDocument);
  const zipDirectory = path.join(out, "zips");
  await mkdir(zipDirectory, { recursive: true });

  let server: ChildProcess | undefined;
  try {
    if (options.startServer) server = await startServer(staging, options.port, options.serverUrl);
    for (const step of directionSteps()) {
      const scene = scenes[step];
      if (scene === undefined) throw new Error(`No staged scene for direction ${step}`);
      const zipPath = path.join(zipDirectory, `${scene}.zip`);
      await bakeOne(options, scene, zipPath);
      const compiledName = `editor-${mapSlug(scene)}`;
      const bytes = await extractPng(zipPath, compiledName);
      const basePng = `editor-${mapSlug(sourceMap)}${PNG_SUFFIX}`;
      await writeFile(path.join(partial, directionPngName(basePng, step)), bytes);
    }
    await Promise.all(
      directionSteps().map(async (step) => {
        const name = directionPngName(`editor-${mapSlug(sourceMap)}${PNG_SUFFIX}`, step);
        await rename(path.join(partial, name), path.join(out, name));
      }),
    );
    if (!options.keepZips) await rm(zipDirectory, { recursive: true, force: true });
    console.log(`wrote ${DIRECTION_COUNT} direction PNGs under ${out}`);
  } finally {
    await stopServer(server);
    await rm(partial, { recursive: true, force: true });
  }
}

const invokedPath = process.argv[1] ? path.resolve(process.argv[1]) : "";
if (invokedPath === fileURLToPath(import.meta.url)) {
  bakeCommand(process.argv.slice(2)).catch((error: unknown) => {
    console.error(error instanceof Error ? (error.stack ?? error.message) : String(error));
    process.exitCode = 1;
  });
}
