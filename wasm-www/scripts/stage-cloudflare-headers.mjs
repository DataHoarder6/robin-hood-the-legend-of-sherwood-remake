import { lstat, readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const HEADER_SOURCE = Object.freeze({
  public: "public-headers.txt",
  runtime: "runtime-headers.txt",
  datadir: "datadir-headers.txt",
  signer: "signer-headers.txt",
});

export async function stageCloudflareHeaders(kind, outputDirectory) {
  const sourceName = HEADER_SOURCE[kind];
  if (sourceName === undefined) {
    throw new Error("header kind must be exactly public, runtime, datadir, or signer");
  }
  const output = resolve(outputDirectory);
  const facts = await lstat(output).catch(() => undefined);
  if (facts === undefined || !facts.isDirectory() || facts.isSymbolicLink()) {
    throw new Error(`static output is not a real directory: ${output}`);
  }
  // One origin input drives the HTML and the headers. Defaults stay upstream's
  // so the Cloudflare workflow is unchanged; an estate build supplies both.
  const signerOrigin =
    process.env.ROBIN_SIGNER_ORIGIN ?? "https://identity.robinhood.phiresky.xyz";
  const gameOrigin = process.env.ROBIN_GAME_ORIGIN ?? "https://robinhood.phiresky.xyz";
  const source = await readFile(resolve(import.meta.dirname, "..", "deploy", sourceName), "utf8");
  await writeFile(
    resolve(output, "_headers"),
    source
      .replaceAll("%ROBIN_SIGNER_ORIGIN%", signerOrigin)
      .replaceAll("%ROBIN_GAME_ORIGIN%", gameOrigin),
  );
}

async function main() {
  const [kind, outputDirectory, extra] = process.argv.slice(2);
  if (kind === undefined || outputDirectory === undefined || extra !== undefined) {
    throw new Error(
      "usage: node scripts/stage-cloudflare-headers.mjs public|runtime|datadir|signer DIST",
    );
  }
  await stageCloudflareHeaders(kind, outputDirectory);
}

const invokedPath = process.argv[1];
if (invokedPath !== undefined && import.meta.url === pathToFileURL(resolve(invokedPath)).href) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  });
}
