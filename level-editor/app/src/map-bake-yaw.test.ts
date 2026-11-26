import test from "node:test";
import assert from "node:assert/strict";
import {
  gameToBakedMap,
  gameToScene,
  sceneToBakedMap,
  yawSinCos,
  type MapCamera,
} from "../../shared/src/scene.ts";
import { sceneToGltf } from "../../shared/src/geometry.ts";
import {
  rasterizeMaskGeometry,
  type MaskTriangle,
} from "../../shared/src/compile-mask-geometry.ts";
import { parseLevel3D } from "../../shared/src/validation.ts";
import { bakeCameraOffset, bakeDepthGroundY } from "./map-bake-render.ts";

const camera = (yaw_deg?: number): MapCamera => ({
  kind: "oblique-orthographic",
  elevation_deg: 35,
  ...(yaw_deg === undefined ? {} : { yaw_deg }),
});
const sin = Math.sin((35 * Math.PI) / 180);
const close = (a: number, b: number) => assert.ok(Math.abs(a - b) < 1e-9, `${a} is not ${b}`);

test("yaw basis keeps the canonical game projection at yaw zero", () => {
  for (const point of [
    [13, -27, 41],
    [0, 0, 0],
    [-50, 200, 0],
  ]) {
    assert.deepEqual(gameToBakedMap(camera(), point), [point[0], point[1] - point[2]]);
  }
  const point: [number, number, number] = [13, -27, 41];
  assert.deepEqual(
    sceneToBakedMap(camera(), gameToScene(camera(), ...point)),
    gameToBakedMap(camera(), point),
  );
});

test("cardinal yaws are exact and share the depth basis with map projection", () => {
  const point: [number, number, number] = [120, -80, 30];
  const world = sceneToGltf(gameToScene(camera(), ...point));
  assert.deepEqual(yawSinCos(camera(0)), [0, 1]);
  assert.deepEqual(yawSinCos(camera(90)), [1, 0]);
  assert.deepEqual(yawSinCos(camera(180)), [0, -1]);
  close(bakeDepthGroundY(camera(), world[0], world[2]), point[1]);
  close(bakeDepthGroundY(camera(90), world[0], world[2]), sin * point[0]);
  close(bakeDepthGroundY(camera(180), world[0], world[2]), -point[1]);
  const [mapX, mapY] = gameToBakedMap(camera(90), point);
  close(mapX, -point[1] / sin);
  close(mapY, sin * point[0] - point[2]);
  // Ground-plane points: screen Y equals the encoded depth, whatever the yaw.
  const ground: [number, number, number] = [120, -80, 0];
  const groundWorld = sceneToGltf(gameToScene(camera(), ...ground));
  for (const yaw of [0, 90, 180]) {
    close(
      gameToBakedMap(camera(yaw), ground)[1],
      bakeDepthGroundY(camera(yaw), groundWorld[0], groundWorld[2]),
    );
  }
});

const rules = {
  layer: 0,
  mask_type: 4,
  character_polyline: null,
  projectile_polyline: null,
  obstacle_indices: [],
};
const triangle: MaskTriangle[] = [
  [
    [10, 20, 0],
    [30, 20, 0],
    [30, 40, 0],
  ],
];

test("mask rasterization is byte-identical at yaw zero and rotates by the yaw basis", () => {
  assert.deepEqual(
    rasterizeMaskGeometry(triangle, rules, false, undefined, camera()),
    rasterizeMaskGeometry(triangle, rules),
  );
  const yawed = rasterizeMaskGeometry(triangle, rules, false, undefined, camera(90));
  assert.deepEqual(yawed[0]!.box_top_left, [Math.floor(-40 / sin), Math.floor(10 * sin)]);
  assert.deepEqual(yawed[0]!.box_size, [
    Math.ceil(-20 / sin) - Math.floor(-40 / sin),
    Math.ceil(30 * sin) - Math.floor(10 * sin),
  ]);
});

test("the bake camera orbits by yaw without changing elevation", () => {
  const distance = 1000;
  const horizontal = Math.cos((35 * Math.PI) / 180) * distance;
  assert.deepEqual(bakeCameraOffset(camera(), distance), [0, sin * distance, horizontal]);
  const east = bakeCameraOffset(camera(90), distance);
  close(east[0], horizontal);
  close(east[1], sin * distance);
  close(east[2], 0);
  const south = bakeCameraOffset(camera(180), distance);
  close(south[0], 0);
  close(south[1], sin * distance);
  close(south[2], -horizontal);
});

test("map camera yaw is validated with zero as the default", () => {
  const document = {
    version: 1,
    map: "York",
    size: [100, 200],
    camera: { ...camera(90) },
    sceneAssets: [],
    groups: [],
    objects: [],
  };
  assert.equal(parseLevel3D(document).camera.yaw_deg, 90);
  assert.equal(parseLevel3D({ ...document, camera: camera() }).camera.yaw_deg, undefined);
  assert.throws(
    () => parseLevel3D({ ...document, camera: { ...camera(), yaw_deg: -1 } }),
    /yaw_deg/,
  );
  assert.throws(
    () => parseLevel3D({ ...document, camera: { ...camera(), yaw_deg: 360 } }),
    /yaw_deg/,
  );
});
