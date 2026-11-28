// Direction-ring naming, scene cloning and ZIP packaging.
//
// Pure Node: nothing here renders. The render half lives in
// `direction-ring-bake.ts`, which drives the repo's existing headless browser
// bake fixture. Keeping the two apart lets the selection/naming contract be
// tested without a browser or an asset library.

import { readFile, mkdir, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { unzipSync, zipSync } from "fflate";
import sharp from "sharp";

export const DIRECTION_COUNT = 24;
export const YAW_STEP_DEG = 15;
const PNG_SUFFIX = ".map.png";
const ZIP_LEVEL = 6;

export interface DirectionDocument {
  map: string;
  camera: { elevation_deg: number; yaw_deg?: number; kind?: string };
  [key: string]: unknown;
}

export interface BaseMapEntry {
  /** Full path inside the ZIP, e.g. Data/Levels/Day/editor-foo.map.png. */
  path: string;
  /** File name without .map.png, e.g. editor-foo. */
  baseName: string;
}

/** Steps 0..23, each a 15-degree yaw from 0 to 345. */
export function directionSteps(): number[] {
  return Array.from({ length: DIRECTION_COUNT }, (_, step) => step);
}

export function directionYaw(step: number): number {
  if (!Number.isSafeInteger(step) || step < 0 || step >= DIRECTION_COUNT)
    throw new Error(`Direction step must be an integer in 0..${DIRECTION_COUNT - 1}: ${step}`);
  return step * YAW_STEP_DEG;
}

/** The scene-file suffix used to get an independently bakeable document. */
export function directionSuffix(step: number): string {
  return `-ring${directionYaw(step) / YAW_STEP_DEG}`;
}

/** Runtime name: step 0 is the base, every other step gets -dir<N>. */
export function directionPngName(basePngName: string, step: number): string {
  directionYaw(step);
  if (!basePngName.endsWith(PNG_SUFFIX))
    throw new Error(`Expected a baked map PNG name ending in ${PNG_SUFFIX}: ${basePngName}`);
  const stem = basePngName.slice(0, -PNG_SUFFIX.length);
  return `${stem}${step === 0 ? "" : `-dir${step}`}${PNG_SUFFIX}`;
}

/** Mirrors map-compile.ts's slug so the baked PNG name matches the mod name. */
export function mapSlug(mapName: string): string {
  const slug = mapName
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return slug || "map";
}

export function directionSceneName(mapName: string, step: number): string {
  return `${mapSlug(mapName)}${directionSuffix(step)}`;
}

function requireDirectionDocument(value: unknown): DirectionDocument {
  if (value === null || typeof value !== "object" || Array.isArray(value))
    throw new Error("Map document must be a JSON object");
  const candidate = value as Record<string, unknown>;
  if (typeof candidate.map !== "string" || !candidate.map)
    throw new Error("Map document must have a non-empty map name");
  if (candidate.camera === null || typeof candidate.camera !== "object")
    throw new Error("Map document must have a camera object");
  return candidate as unknown as DirectionDocument;
}

/** Clone (never mutate) the source document and aim its bake camera at one step. */
export function cloneSceneWithYaw(source: unknown, step: number): DirectionDocument {
  const document = requireDirectionDocument(source);
  const yaw = directionYaw(step);
  const clone = structuredClone(document);
  clone.camera.yaw_deg = yaw;
  clone.map = directionSceneName(document.map, step);
  return clone;
}

export function directionVariants(source: unknown): DirectionDocument[] {
  return directionSteps().map((step) => cloneSceneWithYaw(source, step));
}

/** Find the single base map PNG in an exported mod ZIP. */
export function findBaseMapEntry(
  entries: Record<string, Uint8Array>,
  mapName?: string,
): BaseMapEntry {
  const wanted = mapName ? `${mapSlug(mapName)}.map.png` : undefined;
  if (wanted && !wanted.startsWith("editor-"))
    throw new Error(`Pass the mod map name (the editor-<slug> in the ZIP), got ${mapName}`);
  const matches = Object.keys(entries)
    .filter((entry) => entry.toLowerCase().endsWith(PNG_SUFFIX))
    .filter((entry) => !wanted || path.posix.basename(entry) === wanted);
  if (matches.length === 0)
    throw new Error(`Base map PNG ${wanted ?? "*.map.png"} not found in the mod ZIP`);
  if (matches.length > 1) {
    const day = matches.filter((entry) => entry.split("/").includes("Day"));
    const entry = day.length === 1 ? day[0] : undefined;
    if (entry === undefined)
      throw new Error(`Multiple base map PNGs in the mod ZIP; pass --map: ${matches.join(", ")}`);
    return { path: entry, baseName: path.posix.basename(entry, PNG_SUFFIX) };
  }
  const entry = matches[0];
  if (entry === undefined) throw new Error("Base map PNG not found in the mod ZIP");
  return { path: entry, baseName: path.posix.basename(entry, PNG_SUFFIX) };
}

async function pngSize(bytes: Uint8Array): Promise<[number, number]> {
  const metadata = await sharp(bytes).metadata();
  if (!metadata.width || !metadata.height) throw new Error("Baked PNG has no readable dimensions");
  return [metadata.width, metadata.height];
}

/**
 * Put all 24 baked PNGs into the base mod ZIP. Every other entry is carried
 * through byte-for-byte; the step-0 bake replaces the base PNG so the whole
 * ring comes from one bake run.
 */
export async function mergeDirectionRing(
  baseZip: Uint8Array,
  pngs: readonly Uint8Array[],
  options: { mapName?: string } = {},
): Promise<Uint8Array> {
  if (pngs.length !== DIRECTION_COUNT)
    throw new Error(`Expected ${DIRECTION_COUNT} direction PNGs, got ${pngs.length}`);
  const entries = unzipSync(baseZip);
  const base = findBaseMapEntry(entries, options.mapName);
  const directory = path.posix.dirname(base.path);
  const basePng = pngs[0];
  if (!basePng) throw new Error("Direction 0 PNG is missing");
  const [width, height] = await pngSize(basePng);
  const merged: Record<string, Uint8Array> = { ...entries };
  for (const step of directionSteps()) {
    const bytes = pngs[step];
    if (!bytes) throw new Error(`Direction ${step} PNG is missing`);
    const [pngWidth, pngHeight] = await pngSize(bytes);
    if (pngWidth !== width || pngHeight !== height)
      throw new Error(
        `Direction ${step} PNG is ${pngWidth}x${pngHeight}, expected ${width}x${height}`,
      );
    const name = directionPngName(base.baseName + PNG_SUFFIX, step);
    merged[step === 0 ? base.path : directory === "." ? name : `${directory}/${name}`] = bytes;
  }
  return zipSync(merged, { level: ZIP_LEVEL });
}

/** CLI: `package <base.zip> <png-directory> <out.zip> [--map <mod-map-name>]` */
export async function packageCommand(argv: string[]): Promise<void> {
  const positional = argv.filter((arg) => !arg.startsWith("--"));
  const mapIndex = argv.indexOf("--map");
  const mapName = mapIndex >= 0 ? argv[mapIndex + 1] : undefined;
  if (mapIndex >= 0 && (!mapName || mapName.startsWith("--")))
    throw new Error("--map needs a value");
  if (positional.length !== 3)
    throw new Error("usage: package <base.zip> <png-directory> <out.zip> [--map <mod-map-name>]");
  const [baseZipPath, pngDirectory, outZipPath] = positional as [string, string, string];
  const baseZip = new Uint8Array(await readFile(baseZipPath));
  const base = findBaseMapEntry(unzipSync(baseZip), mapName);
  const pngs = await Promise.all(
    directionSteps().map(async (step) => {
      const name = directionPngName(base.baseName + PNG_SUFFIX, step);
      return new Uint8Array(await readFile(path.join(pngDirectory, name)));
    }),
  );
  const merged = await mergeDirectionRing(baseZip, pngs, { mapName });
  await mkdir(path.dirname(outZipPath), { recursive: true });
  await writeFile(outZipPath, merged);
  console.log(`wrote ${outZipPath}: ${DIRECTION_COUNT} directions, ${merged.length} ZIP bytes`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  if (args[0] === "package") args.shift();
  packageCommand(args).catch((error: unknown) => {
    console.error(error instanceof Error ? (error.stack ?? error.message) : String(error));
    process.exitCode = 1;
  });
}
