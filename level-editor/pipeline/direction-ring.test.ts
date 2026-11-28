import assert from "node:assert/strict";
import test from "node:test";
import { strToU8, unzipSync, zipSync } from "fflate";
import sharp from "sharp";
import {
  DIRECTION_COUNT,
  directionPngName,
  directionSceneName,
  directionSteps,
  directionVariants,
  directionYaw,
  findBaseMapEntry,
  mergeDirectionRing,
} from "./direction-ring.ts";

const onePixelPng = new Uint8Array(
  Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
    "base64",
  ),
);

const sourceDocument = {
  version: 1,
  map: "Lincoln",
  size: [1920, 1088],
  camera: { kind: "oblique-orthographic", elevation_deg: 35 },
  objects: [{ id: "x", node: "x", transform: { dx: 0, dy: 0, dz: 0, rot_deg: 0 } }],
};

test("yaw selection is exactly 0..345 in 15-degree steps", () => {
  const steps = directionSteps();
  assert.equal(steps.length, DIRECTION_COUNT);
  assert.deepEqual(
    steps.map(directionYaw),
    Array.from({ length: 24 }, (_, step) => step * 15),
  );
  assert.equal(directionYaw(23), 345);
  for (const invalid of [-1, 24, 1.5, Number.NaN])
    assert.throws(() => directionYaw(invalid), /Direction step/);
});

test("PNG names keep step 0 and suffix steps 1..23", () => {
  assert.equal(directionPngName("editor-lincoln.map.png", 0), "editor-lincoln.map.png");
  assert.equal(directionPngName("editor-lincoln.map.png", 1), "editor-lincoln-dir1.map.png");
  assert.equal(directionPngName("editor-lincoln.map.png", 23), "editor-lincoln-dir23.map.png");
  assert.throws(() => directionPngName("editor-lincoln.png", 1), /baked map PNG/);
  assert.throws(() => directionPngName("editor-lincoln.map.png", 24), /Direction step/);
});

test("cloned scenes carry one yaw each without mutating the source", () => {
  const before = structuredClone(sourceDocument);
  const variants = directionVariants(sourceDocument);
  assert.equal(variants.length, 24);
  assert.deepEqual(sourceDocument, before);
  variants.forEach((document, step) => {
    assert.equal(document.camera.yaw_deg, step * 15);
    assert.equal(document.map, directionSceneName("Lincoln", step));
    assert.deepEqual(document.objects, before.objects);
  });
  assert.equal(new Set(variants.map((document) => document.map)).size, 24);
});

test("the base map PNG is found in a mod ZIP, preferring Day", () => {
  const entries = {
    "details.json": strToU8("{}"),
    "Data/Levels/Fog/editor-lincoln.map.png": onePixelPng,
    "Data/Levels/Day/editor-lincoln.map.png": onePixelPng,
    "Data/Levels/Day/editor-lincoln.occlusion-depth.png": onePixelPng,
  };
  assert.equal(findBaseMapEntry(entries).path, "Data/Levels/Day/editor-lincoln.map.png");
  assert.equal(findBaseMapEntry(entries, "editor-lincoln").baseName, "editor-lincoln");
  assert.throws(() => findBaseMapEntry(entries, "editor-york"), /not found/);
});

test("packaging keeps every other ZIP entry and emits all 24 PNGs", async () => {
  const details = strToU8('{"hackable_missions":[]}');
  const depth = strToU8("depth-placeholder");
  const basePath = "Data/Levels/Day/editor-lincoln.map.png";
  const baseZip = zipSync({
    "details.json": details,
    [basePath]: onePixelPng,
    "Data/Levels/Day/editor-lincoln.occlusion-depth.png": depth,
  });
  const pngs = Array.from({ length: DIRECTION_COUNT }, () => onePixelPng);
  const result = await mergeDirectionRing(baseZip, pngs);
  const files = unzipSync(result);
  assert.deepEqual(files["details.json"], details);
  assert.deepEqual(files["Data/Levels/Day/editor-lincoln.occlusion-depth.png"], depth);
  assert.deepEqual(files[basePath], onePixelPng);
  for (const step of directionSteps()) {
    const name = directionPngName("editor-lincoln.map.png", step);
    assert.deepEqual(files[`Data/Levels/Day/${name}`], onePixelPng, `direction ${step}`);
  }
  assert.equal(Object.keys(files).length, 3 + DIRECTION_COUNT - 1);
  await assert.rejects(mergeDirectionRing(baseZip, pngs.slice(1)), /Expected 24 direction PNGs/);
  const largerPng = new Uint8Array(
    await sharp({ create: { width: 2, height: 2, channels: 4, background: "#000000" } })
      .png()
      .toBuffer(),
  );
  const mismatched = [...pngs];
  mismatched[7] = largerPng;
  await assert.rejects(mergeDirectionRing(baseZip, mismatched), /Direction 7 PNG is 2x2/);
});
