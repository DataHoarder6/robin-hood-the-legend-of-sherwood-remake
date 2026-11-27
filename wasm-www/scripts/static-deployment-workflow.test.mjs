import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { chmod, mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import test from "node:test";
import { runtimeBuildPlan } from "./build-runtime.mjs";

const repository = resolve(import.meta.dirname, "../..");

async function repositoryFile(path) {
  return readFile(resolve(repository, path), "utf8");
}

function ordered(text, markers) {
  let previous = -1;
  for (const marker of markers) {
    const index = text.indexOf(marker);
    assert(index >= 0, `missing ${marker}`);
    assert(index > previous, `${marker} is out of order`);
    previous = index;
  }
}

// The CLI the workflows pin must match the wasm-bindgen crate the build locks.
// Derive the version from Cargo.lock so a re-pin cannot leave a stale literal
// behind in this assertion.
async function pinnedBindgenVersion() {
  const lock = await repositoryFile("Cargo.lock");
  const match = lock.match(/name = "wasm-bindgen"\nversion = "(\d+\.\d+\.\d+)"/u);
  assert(match, "cannot find the wasm-bindgen version in Cargo.lock");
  return match[1];
}

test("Cloudflare deploy verifies builds, deploys signer before public, then reconciles routes", async () => {
  const script = await repositoryFile("wasm-www/scripts/deploy-cloudflare.sh");
  ordered(script, [
    "pnpm verify:public",
    "pnpm verify:signer",
    "deploy_worker signer robinhood-identity-signer",
    "deploy_worker public robinhood-public-site",
    "sync-cloudflare-routes.mjs --apply",
    "sync-cloudflare-routes.mjs --check",
    "pnpm smoke:cloudflare",
  ]);
  const workflow = await repositoryFile(".github/workflows/deploy-static-workers.yml");
  assert.match(workflow, /^\s*workflow_dispatch:/mu);
  assert.match(workflow, /run: wasm-www\/scripts\/deploy-cloudflare\.sh$/mu);
  const pinnedVersionCheck = `wasm-bindgen ${await pinnedBindgenVersion()}`;
  for (const text of [
    workflow,
    await repositoryFile(".github/workflows/build-static-runtime.yml"),
  ]) {
    assert.match(text, /scripts\/install_pinned_wasm_bindgen\.sh/u);
    assert(text.includes(pinnedVersionCheck), `workflows must check '${pinnedVersionCheck}'`);
    assert.doesNotMatch(text, /cargo install wasm-bindgen-cli|wrangler pages|github-pages/iu);
  }
});

test("jautis publisher changes trigger the static deployment workflow", async () => {
  const workflow = await repositoryFile(".github/workflows/deploy-static-workers.yml");
  for (const path of ["scripts/deploy_web_jautis.sh", "scripts/release-parity-check.sh"]) {
    assert.match(
      workflow,
      new RegExp(`^\\s+- ${path.replaceAll(".", "\\.")}$`, "mu"),
      `${path} is not in the PR paths filter`,
    );
  }
});

test("runtime workflow stages the artifact before uploading it", async () => {
  const text = await repositoryFile(".github/workflows/build-static-runtime.yml");
  ordered(text, ["node wasm-www/scripts/stage-runtime-addition.mjs", "actions/upload-artifact@"]);
  const plan = runtimeBuildPlan();
  const nameIndex = plan.bindgenArgs.indexOf("--out-name");
  assert(nameIndex >= 0);
  assert.equal(plan.bindgenArgs[nameIndex + 1], "robin");
});

test("shipping datadir wrapper selects the locked converter package and tools feature", async () => {
  const text = await repositoryFile("scripts/build_web_shipping_datadir.sh");
  assert.match(
    text,
    /^cargo build --locked --release -p robin_app --bin convert_datadir --features tools$/mu,
  );
  assert.doesNotMatch(text, /^cargo build --release --bin convert_datadir$/mu);
});

// D20 (#127): the private root fetches /campaign.json?build=<short>. Deployment
// stages the operator-managed binding and gates it under loopback HTTP before
// the atomic swap. These tests execute the gate's actual shell code against
// generated fixtures: no live origin, licensed corpus or secret is involved.
const gateBegin = "# CAMPAIGN_GATE_BEGIN";
const gateEnd = "# CAMPAIGN_GATE_END";
const selector = "abc1234def56";
const corpusPath = `/datadirs/full/${"b".repeat(64)}/datadir.bin`;

function delimited(script, beginMarker, endMarker, label) {
  const begin = script.indexOf(beginMarker);
  const end = script.indexOf(endMarker);
  assert(begin >= 0 && end > begin, `deploy_web_jautis.sh has no delimited ${label}`);
  return script.slice(begin + beginMarker.length, end);
}

function campaignGateSource(script) {
  return delimited(script, gateBegin, gateEnd, "campaign gate");
}

// Run shell text in an isolated directory with async stdio: spawnSync would
// block the event loop that hosts the gate's fixture HTTP server.
async function runBashHarness(text, env = process.env) {
  const dir = await mkdtemp(join(tmpdir(), "rh-shell-harness-"));
  try {
    const file = join(dir, "harness.sh");
    await writeFile(file, `#!/usr/bin/env bash\n${text}\n`, { mode: 0o700 });
    const child = spawn("bash", [file], { env });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk) => (stdout += chunk));
    child.stderr.on("data", (chunk) => (stderr += chunk));
    const status = await new Promise((done) => {
      child.on("error", () => done(-1));
      child.on("close", done);
    });
    return { status, stdout, stderr };
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}

function binding(overrides = {}) {
  return { url: corpusPath, sha256: "a".repeat(64), byteLength: 32, ...overrides };
}

async function stageCorpus(root, data, split) {
  await mkdir(join(root, dirname(corpusPath)), { recursive: true });
  if (split === 1) {
    await writeFile(join(root, corpusPath), data);
    return;
  }
  const chunk = Math.ceil(data.byteLength / split);
  for (let index = 0; index < split; index += 1) {
    await writeFile(
      join(root, `${corpusPath}.part${index}`),
      data.subarray(index * chunk, (index + 1) * chunk),
    );
  }
}

async function serveStaged(root, expectedSelector, run, serveOverride) {
  const server = createServer(async (request, response) => {
    const url = new URL(request.url, "http://127.0.0.1");
    if (url.pathname === "/campaign.json" && url.searchParams.get("build") !== expectedSelector) {
      response.writeHead(404);
      response.end("no binding for this build");
      return;
    }
    const file =
      serveOverride && url.pathname === "/campaign.json"
        ? join(root, "binding-override.json")
        : join(root, url.pathname);
    try {
      const body = await readFile(file);
      response.writeHead(200);
      response.end(body);
    } catch {
      response.writeHead(404);
      response.end("not found");
    }
  });
  await new Promise((done) => server.listen(0, "127.0.0.1", done));
  try {
    return await run(`http://127.0.0.1:${server.address().port}`);
  } finally {
    await new Promise((done) => server.close(done));
  }
}

async function gateCase({
  mutate,
  gateSelector = selector,
  corpusSize = 32,
  split = 1,
  serveOverride = false,
} = {}) {
  const script = await repositoryFile("scripts/deploy_web_jautis.sh");
  const root = await mkdtemp(join(tmpdir(), "rh-campaign-gate-"));
  try {
    const data = Buffer.alloc(corpusSize, 0x5a);
    const sha256 = createHash("sha256").update(data).digest("hex");
    await stageCorpus(root, data, split);
    await writeFile(
      join(root, "campaign.json"),
      JSON.stringify(binding({ sha256, byteLength: corpusSize })),
    );
    if (serveOverride) {
      await writeFile(
        join(root, "binding-override.json"),
        JSON.stringify(binding({ sha256, byteLength: corpusSize, url: `${corpusPath}.tampered` })),
      );
    }
    if (mutate) await mutate(root, { sha256 });
    return await serveStaged(
      root,
      selector,
      async (gateBase) =>
        runBashHarness(`set -euo pipefail
gate_base='${gateBase}'
gate_body=$(mktemp)
gate_headers=$(mktemp)
campaign_parts=$(mktemp)
trap 'rm -f -- "$gate_body" "$gate_headers" "$campaign_parts"' EXIT
gate_fail() { echo "GATE_FAIL: $*" >&2; exit 1; }
${campaignGateSource(script)}
gate_campaign_binding '${root}' '${gateSelector}'
echo GATE_PASSED
`),
      serveOverride,
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

test("jautis deploy stages the operator binding and gates it before the swap", async () => {
  const script = await repositoryFile("scripts/deploy_web_jautis.sh");
  assert.match(script, /CAMPAIGN_BINDING_PATH:\?/u, "binding path must be explicitly required");
  assert.match(
    script,
    /cp "\$campaign_binding" "\$public_new\/campaign\.json"/u,
    "binding must be staged at the served root",
  );
  const call = 'gate_campaign_binding "$public_new" "$campaign_selector"';
  assert(script.includes(call), "campaign gate is not called");
  assert.match(
    script,
    /\[ "\$campaign_selector" = "\$short" \]/u,
    "the build selector must select the deployed build",
  );
  const swap = script.indexOf("# Atomic swap");
  assert(swap >= 0 && script.indexOf(call) < swap, "campaign gate must run before the swap");
  const body = campaignGateSource(script);
  for (const marker of [
    "/campaign.json?build=",
    "cmp -s",
    "^/datadirs/full/",
    "sha256sum",
    "byteLength",
  ]) {
    assert(body.includes(marker), `campaign gate has no ${marker} check`);
  }
});

test("jautis campaign gate accepts a valid same-origin binding", async () => {
  const result = await gateCase();
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /GATE_PASSED/u);
});

test("jautis campaign gate accepts a valid multi-part binding", async () => {
  const result = await gateCase({ corpusSize: 25 * 1024 * 1024 + 1, split: 2 });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /GATE_PASSED/u);
});

/** @type {Array<[string, Parameters<typeof gateCase>[0]]>} */
const rejected = [
  ["a missing binding", { mutate: async (root) => rm(join(root, "campaign.json")) }],
  [
    "malformed JSON",
    { mutate: async (root) => writeFile(join(root, "campaign.json"), "{not json") },
  ],
  [
    "a binding with the wrong shape",
    { mutate: async (root) => writeFile(join(root, "campaign.json"), "[]") },
  ],
  [
    "a cross-origin URL",
    {
      mutate: async (root, { sha256 }) =>
        writeFile(
          join(root, "campaign.json"),
          JSON.stringify(binding({ sha256, url: `https://elsewhere.example${corpusPath}` })),
        ),
    },
  ],
  [
    "a wrong sha256",
    {
      mutate: async (root) =>
        writeFile(join(root, "campaign.json"), JSON.stringify(binding({ sha256: "0".repeat(64) }))),
    },
  ],
  [
    "a wrong byteLength",
    {
      mutate: async (root, { sha256 }) =>
        writeFile(join(root, "campaign.json"), JSON.stringify(binding({ sha256, byteLength: 33 }))),
    },
  ],
  [
    "a missing part",
    {
      corpusSize: 25 * 1024 * 1024 + 1,
      split: 2,
      mutate: async (root) => rm(join(root, `${corpusPath}.part1`)),
    },
  ],
  ["a mismatched build selector", { gateSelector: "deadbeefdead" }],
  ["served bytes that differ from the staged binding", { serveOverride: true }],
  [
    "a url with a trailing newline",
    {
      mutate: async (root, { sha256 }) =>
        writeFile(
          join(root, "campaign.json"),
          JSON.stringify(binding({ sha256, url: `${corpusPath}\n` })),
        ),
    },
  ],
  [
    "a sha256 with a trailing newline",
    {
      mutate: async (root, { sha256 }) =>
        writeFile(join(root, "campaign.json"), JSON.stringify(binding({ sha256: `${sha256}\n` }))),
    },
  ],
];

for (const [label, options] of rejected) {
  test(`jautis campaign gate fails closed on ${label}`, async () => {
    const result = await gateCase(options);
    assert.notEqual(result.status, 0, `gate accepted ${label}`);
    assert.match(result.stderr, /GATE_FAIL:/u);
    assert.doesNotMatch(result.stdout, /GATE_PASSED/u);
  });
}

test("jautis deploy refuses an absent or nonexistent binding before any remote work", async () => {
  const script = await repositoryFile("scripts/deploy_web_jautis.sh");
  const input = delimited(
    script,
    "# CAMPAIGN_INPUT_BEGIN",
    "# CAMPAIGN_INPUT_END",
    "binding input",
  );
  assert(
    script.indexOf("# CAMPAIGN_INPUT_BEGIN") < script.indexOf('git -C "$repo" fetch'),
    "the binding path must be checked before fetch, SSH and staging",
  );
  const harness = `set -euo pipefail
git() { echo CALLED_GIT >&2; }
ssh() { echo CALLED_SSH >&2; }
scp() { echo CALLED_SCP >&2; }
cp() { echo CALLED_CP >&2; }
${input}
echo INPUT_PASSED
`;
  const noRemoteWork = (result) => {
    assert.doesNotMatch(result.stderr, /CALLED_(GIT|SSH|SCP|CP)/u);
    assert.doesNotMatch(result.stdout, /INPUT_PASSED/u);
  };
  const unsetEnv = { ...process.env };
  delete unsetEnv.CAMPAIGN_BINDING_PATH;
  const unset = await runBashHarness(harness, unsetEnv);
  assert.notEqual(unset.status, 0, "unset binding path was accepted");
  assert.match(unset.stderr, /CAMPAIGN_BINDING_PATH/u);
  noRemoteWork(unset);

  const dir = await mkdtemp(join(tmpdir(), "rh-campaign-input-"));
  try {
    const missing = await runBashHarness(harness, {
      ...process.env,
      CAMPAIGN_BINDING_PATH: join(dir, "absent.json"),
    });
    assert.notEqual(missing.status, 0, "nonexistent binding path was accepted");
    assert.match(missing.stderr, /no binding at/u);
    noRemoteWork(missing);

    const present = join(dir, "campaign.json");
    await writeFile(present, "{}");
    const ok = await runBashHarness(harness, {
      ...process.env,
      CAMPAIGN_BINDING_PATH: present,
    });
    assert.equal(ok.status, 0, ok.stderr);
    assert.match(ok.stdout, /INPUT_PASSED/u);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test("jautis deploy stages campaign.json Caddy-readable with served-file ownership", async () => {
  const script = await repositoryFile("scripts/deploy_web_jautis.sh");
  const stage = delimited(
    script,
    "# CAMPAIGN_STAGE_BEGIN",
    "# CAMPAIGN_STAGE_END",
    "binding staging",
  );
  assert(
    script.indexOf("# CAMPAIGN_STAGE_BEGIN") < script.indexOf("# CAMPAIGN_GATE_BEGIN"),
    "the binding must be staged before the gate fetches it",
  );
  const root = await mkdtemp(join(tmpdir(), "rh-campaign-stage-"));
  try {
    const source = join(root, "operator-campaign.json");
    const publicNew = join(root, "public-new");
    await mkdir(publicNew);
    await writeFile(source, "{}");
    await chmod(source, 0o600);
    const reference = join(publicNew, "index.html");
    await writeFile(reference, "<html></html>");
    await chmod(reference, 0o640);
    const result = await runBashHarness(`set -euo pipefail
campaign_binding='${source}'
public_new='${publicNew}'
${stage}
echo STAGE_PASSED
`);
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /STAGE_PASSED/u);
    const [staged, ref, operator] = await Promise.all([
      stat(join(publicNew, "campaign.json")),
      stat(reference),
      stat(source),
    ]);
    assert.equal((operator.mode & 0o777).toString(8), "600", "fixture source must be private");
    assert.equal((staged.mode & 0o777).toString(8), "644", "Caddy must read the staged binding");
    assert.equal(staged.uid, ref.uid, "stage must match an already served file's owner");
    assert.equal(staged.gid, ref.gid, "stage must match an already served file's group");
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("deploy part thresholds and cap match parseHostedReplayContent", async () => {
  const script = await repositoryFile("scripts/deploy_web_jautis.sh");
  const gate = campaignGateSource(script);
  const client = await repositoryFile("wasm-www/src/run-replay.ts");
  const limits = [...client.matchAll(/content\.byteLength > (\d+) \* 1024 \* 1024/gu)].map(
    (match) => Number(match[1]) * 1024 * 1024,
  );
  assert.equal(limits.length, 2, "expected the client's cap and part threshold");
  const [cap, partThreshold] = limits;
  const chunkMatch = client.match(/content\.byteLength \/ \((\d+) \* 1024 \* 1024\)/u);
  assert(chunkMatch, "no client part chunk size");
  const chunk = Number(chunkMatch[1]) * 1024 * 1024;
  assert(gate.includes(`<= ${cap}`), `jq validation must enforce the client cap ${cap}`);
  assert(gate.includes(`-le ${cap}`), `shell must enforce the client cap ${cap}`);
  assert(
    gate.includes(`-gt ${partThreshold}`),
    `shell must use the client threshold ${partThreshold}`,
  );
  assert(
    gate.includes(`bytes + ${chunk - 1}) / ${chunk}`),
    `shell must use the client chunk ${chunk}`,
  );
});
