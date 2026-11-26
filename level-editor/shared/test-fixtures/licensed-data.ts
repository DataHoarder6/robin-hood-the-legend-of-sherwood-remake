import { existsSync } from "node:fs";
import { resolve } from "node:path";

/** Named reason for tests that need operator-supplied licensed game data.
 *
 * The repository carries no licensed mission sources or captured fixture
 * bundles. Those tests must skip explicitly rather than fail on ENOENT or
 * invent a substitute; every assertion that can run on synthetic data still
 * runs. */
export const LICENSED_DATA_REASON = "requires operator-supplied licensed game data";

/** Returns a `node:test` `skip` value when any required licensed file is absent.
 *
 * `false` means the data is present, so the test runs unchanged. */
export function skipWithoutLicensedData(root: string, ...relativePaths: string[]): string | false {
  const missing = relativePaths.filter((path) => !existsSync(resolve(root, path)));
  return missing.length ? `${LICENSED_DATA_REASON}: missing ${missing.join(", ")}` : false;
}
