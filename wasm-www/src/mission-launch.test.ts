import assert from "node:assert/strict";
import test from "node:test";
import { campaignRequested, fullGameContentBuild } from "./mission-launch.ts";

test("full-game mission launch selects matching hosted content without overriding replay content", () => {
  assert.equal(fullGameContentBuild("latest", "full", null, false), "latest");
  assert.equal(fullGameContentBuild("latest", "demo", null, false), null);
  assert.equal(fullGameContentBuild("latest", null, null, false), null);
  assert.equal(fullGameContentBuild("latest", "full", null, true), null);
  assert.equal(
    fullGameContentBuild("latest", "demo", { edition: "full", runtimeBuild: "recorded" }, false),
    "recorded",
  );
  assert.equal(
    fullGameContentBuild("latest", "full", { edition: "demo", runtimeBuild: "recorded" }, false),
    null,
  );
});

test("only an explicit campaign request on a private path boots the full campaign", () => {
  const path = (value: string) => new URL(`https://robin.jautis.lt${value}`);
  const token = `/play/${"a".repeat(32)}/`;
  assert.equal(campaignRequested(path(token), null, false), true);
  assert.equal(campaignRequested(path("/"), "campaign", false), false);
  assert.equal(campaignRequested(path("/"), null, false), false);
  assert.equal(campaignRequested(path(token), null, true), false);
  assert.equal(campaignRequested(path("/play/not-a-token/"), "campaign", false), false);
  assert.equal(campaignRequested(path(token), "demo", false), false);
});
