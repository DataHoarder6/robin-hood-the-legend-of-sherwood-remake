// Resolve the Demo datadir a published build pins in its manifest.
//
// The check used to compare the manifest URL against a literal upstream
// hostname while claiming to enforce same-origin. That rejected a correct
// manifest on every deployment other than upstream's, so the words and the
// code disagreed. This is the pure part: given a manifest entry, the page's
// origin and the asset base, decide where the browser loads the datadir from.

export type DemoDatadirIdentity = { readonly sha256: string; readonly byteLength: number };

export class DemoDatadirError extends Error {}

export function resolvePublishedDemoDatadir(
    demo: Record<string, unknown> | undefined,
    { pageOrigin, binariesBase, buildShort }: {
        pageOrigin: string;
        binariesBase: string;
        buildShort: string;
    },
): { url: string; identity: DemoDatadirIdentity } {
    const url = demo?.url;
    const sha256 = demo?.sha256;
    const byteLength = demo?.byteLength;
    let parsed: URL | null = null;
    try {
        if (typeof url === 'string') parsed = new URL(url, pageOrigin);
    } catch {
        parsed = null;
    }
    if (!parsed || parsed.origin !== new URL(pageOrigin).origin
        || !parsed.pathname.startsWith('/datadirs/')
        || typeof sha256 !== 'string' || !/^[0-9a-f]{64}$/.test(sha256)
        || typeof byteLength !== 'number' || !Number.isSafeInteger(byteLength)) {
        throw new DemoDatadirError(
            `build ${buildShort} manifest does not pin a same-origin Demo datadir`,
        );
    }
    const base = new URL(binariesBase);
    return {
        url: `${base.origin}${parsed.pathname}${parsed.search}${parsed.hash}`,
        identity: { sha256, byteLength },
    };
}
