import { readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { DEPLOYMENT } from "./verify-cloudflare-deployment.mjs";
import {
  CLOUDFLARE_ASSET_BYTES_LIMIT,
  DEMO_PATH,
  RETAINED_DEMO_GENERATIONS,
  verifyDatadirCorpus,
} from "./verify-datadir-corpus.mjs";

/**
 * `datadir-release.json` names the current Demo datadir object of a release
 * staging directory. It is written when a datadir generation is (re)built and
 * read by runtime staging, which copies it into `manifest.json`
 * `multiplayerContent.demo`.
 */
export const DATADIR_RELEASE_SCHEMA = 1;
const KEYS = ["schema", "url", "sha256", "byte_length", "native_content_sha256"];
const DIGEST = /^[0-9a-f]{64}$/u;

// One origin input for the published Demo URL. Cloudflare CI keeps the
// upstream default; an estate publisher sets ROBIN_PUBLIC_ORIGIN to the origin
// that actually serves the bytes, so the release it emits is same-origin with
// the site that will load it. Validate it here, like ROBIN_SIGNER_ORIGIN in
// verify-cloudflare-deployment.mjs, rather than splicing an arbitrary string.
export const DEFAULT_PUBLIC_ORIGIN = DEPLOYMENT.publicOrigin;
const configuredOrigin = process.env.ROBIN_PUBLIC_ORIGIN;
let publicOrigin;
try {
  publicOrigin = new URL(configuredOrigin ?? DEFAULT_PUBLIC_ORIGIN).origin;
} catch {
  throw new Error(`ROBIN_PUBLIC_ORIGIN is not a URL: ${configuredOrigin}`);
}
export const PUBLIC_ORIGIN = publicOrigin;
export const CURRENT_DEMO_URL = `${PUBLIC_ORIGIN}/${DEMO_PATH}`;

export function parseDatadirRelease(
  value,
  label = "datadir-release.json",
  expectedUrl = CURRENT_DEMO_URL,
) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  const actual = Object.keys(value).sort();
  if (JSON.stringify(actual) !== JSON.stringify([...KEYS].sort())) {
    throw new Error(`${label} has unexpected keys: ${actual.join(", ")}`);
  }
  if (value.schema !== DATADIR_RELEASE_SCHEMA)
    throw new Error(`${label} schema must be ${DATADIR_RELEASE_SCHEMA}`);
  // Runtime staging always binds the generation this checkout publishes; a
  // stale release file after a DEMO_ROOT bump is a real release mistake.
  if (value.url !== expectedUrl) {
    throw new Error(
      `${label} url ${JSON.stringify(value.url)} is not the current Demo generation ${expectedUrl}`,
    );
  }
  if (!DIGEST.test(value.sha256) || !DIGEST.test(value.native_content_sha256)) {
    throw new Error(`${label} digests must be lowercase SHA-256`);
  }
  if (
    !Number.isSafeInteger(value.byte_length) ||
    value.byte_length <= 0 ||
    value.byte_length > CLOUDFLARE_ASSET_BYTES_LIMIT
  ) {
    throw new Error(`${label} byte_length must be between 1 byte and 25 MiB`);
  }
  return Object.freeze({ ...value });
}

export async function readDatadirRelease(path) {
  let value;
  try {
    value = JSON.parse(await readFile(path, "utf8"));
  } catch (error) {
    throw new Error(
      `cannot read ${path}: ${error instanceof Error ? error.message : String(error)}`,
      { cause: error },
    );
  }
  return parseDatadirRelease(value, path);
}

async function writeRelease(release, output) {
  await writeFile(output, `${JSON.stringify(parseDatadirRelease(release), null, 2)}\n`, {
    flag: "wx",
  });
  return release;
}

/** Describe the current Demo generation of an assembled datadir corpus. */
export async function writeDatadirRelease({
  root,
  output,
  retainedGenerations = RETAINED_DEMO_GENERATIONS,
}) {
  const { demo } = await verifyDatadirCorpus(root, { retainedGenerations });
  return writeRelease(
    {
      schema: DATADIR_RELEASE_SCHEMA,
      // The corpus verifier labels the object with upstream's origin. The
      // release must name the origin that publishes it, which is the same
      // input parseDatadirRelease and the manifest verify gate use.
      url: CURRENT_DEMO_URL,
      sha256: demo.datadir_sha256,
      byte_length: demo.datadir_byte_length,
      native_content_sha256: demo.native_content_sha256,
    },
    output,
  );
}

async function main() {
  const [mode, ...args] = process.argv.slice(2);
  if (mode === "write" && args.length === 2) {
    const release = await writeDatadirRelease({ root: args[0], output: args[1] });
    console.log(
      `wrote ${args[1]}: ${release.url} ${release.sha256} (${release.byte_length} bytes)`,
    );
  } else if (mode === "current" && args.length === 4) {
    // For CI runtime builds, which receive the Demo identity as inputs.
    const release = await writeRelease(
      {
        schema: DATADIR_RELEASE_SCHEMA,
        url: CURRENT_DEMO_URL,
        sha256: args[0],
        byte_length: Number(args[1]),
        native_content_sha256: args[2],
      },
      args[3],
    );
    console.log(`wrote ${args[3]}: ${release.url}`);
  } else {
    throw new Error(
      "usage: datadir-release.mjs write DATADIR_CORPUS OUTPUT | current SHA256 BYTE_LENGTH NATIVE_CONTENT_SHA256 OUTPUT",
    );
  }
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  });
}
