import { parseHostedReplayContent } from "./run-replay.ts";

const CAPABILITY_PATH = /^\/play\/([0-9a-f]{32})\/$/u;

/** The private site boots the campaign from its root; the old capability URL
 * stays readable for links already shared. */
export function privateBindingPath(pathname: string): string | undefined {
  if (pathname === "/" || pathname === "/index.html") return "/campaign.json";
  const token = CAPABILITY_PATH.exec(pathname)?.[1];
  return token === undefined ? undefined : `/play/${token}/campaign.json`;
}

export type PrivateCampaignContent = {
  readonly url: string;
  readonly sha256: string;
  readonly byteLength: number;
  readonly parts: readonly string[];
};

/** Resolve the digest-pinned full corpus bound to a private capability URL.
 * A normal page never fetches it and a bad binding is a launch error, never a
 * silent fallback to the Demo generation. */
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
