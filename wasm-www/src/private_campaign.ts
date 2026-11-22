import { parseHostedReplayContent } from "./run-replay.ts";

const CAPABILITY_PATH = /^\/play\/([0-9a-f]{32})\/$/u;

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
  const token = CAPABILITY_PATH.exec(pageUrl.pathname)?.[1];
  if (token === undefined) return undefined;
  const bindingUrl = `/play/${token}/campaign.json?build=${encodeURIComponent(latestBuild)}`;
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
