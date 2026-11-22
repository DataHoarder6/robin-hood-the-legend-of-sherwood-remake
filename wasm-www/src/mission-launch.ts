const PRIVATE_CAMPAIGN_PATH = /^\/play\/[0-9a-f]{32}\/$/u;

/** A replay never selects the private campaign, nor does an ordinary page
 * without an explicit request. The capability path is itself a request. */
export function campaignRequested(
  pageUrl: URL,
  requestedEdition: string | null,
  hasSharedReplay: boolean,
): boolean {
  if (hasSharedReplay || requestedEdition === "demo") return false;
  return PRIVATE_CAMPAIGN_PATH.test(pageUrl.pathname);
}

/** A replay's edition takes precedence over ordinary game-launch options. */
export function fullGameContentBuild(
  latestBuild: string,
  requestedEdition: string | null,
  recordedRun: { readonly edition: "full" | "demo"; readonly runtimeBuild: string } | null,
  hasSharedReplay: boolean,
): string | null {
  if (recordedRun !== null) return recordedRun.edition === "full" ? recordedRun.runtimeBuild : null;
  return !hasSharedReplay && requestedEdition === "full" ? latestBuild : null;
}
