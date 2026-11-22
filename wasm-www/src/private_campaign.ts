import { parseHostedReplayContent } from "./run-replay.ts";

/** The private site has exactly one campaign entry point: the origin root. */
export function privateBindingPath(pathname: string): string | undefined {
  return pathname === "/" || pathname === "/index.html" ? "/campaign.json" : undefined;
}

export type PrivateCampaignContent = {
  readonly url: string;
  readonly sha256: string;
  readonly byteLength: number;
  readonly parts: readonly string[];
};

/** Resolve the digest-pinned full corpus bound to the private origin root.
 * A bad binding is a launch error, never a silent fallback to Demo. */
export async function resolvePrivateCampaignContent(
  pageUrl: URL,
  latestBuild: string,
  fetchJson: <T>(url: string, signal: AbortSignal) => Promise<T>,
  signal: AbortSignal,
): Promise<PrivateCampaignContent | undefined> {
  const binding = privateBindingPath(pageUrl.pathname);
  if (binding === undefined) return undefined;
  const bindingUrl = `${binding}?build=${encodeURIComponent(latestBuild)}`;
  const raw = await fetchJson<unknown>(bindingUrl, signal);
  try {
    const content = parseHostedReplayContent(raw, pageUrl.origin);
    return {
      url: content.url,
      sha256: content.sha256,
      byteLength: content.byteLength,
      parts: content.parts,
    };
  } catch (error) {
    throw new Error("private campaign content binding is invalid", { cause: error });
  }
}
