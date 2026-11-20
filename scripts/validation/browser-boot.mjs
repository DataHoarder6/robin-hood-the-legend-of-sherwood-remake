// A browser seat chooses its runtime from the host's own build identity. The
// shell reads the invitation's engine version, derives `/wasm/<short>/`, and
// fetches that manifest before anything else. A host whose artifact was never
// published therefore dies on a 404 before a ticket can be installed — the
// failure has to be named at the host, not at a page that never loaded.

export const BOOT_ARTIFACT_MISSING = short =>
    `host ${short} has no published browser artifact; rebuild and stage it under /wasm/${short}/`;

/** The 12-hex prefix clap prints after '+' in `robin --version`. */
export function hostShortFromVersion(output) {
    const text = String(output).trim();
    const match = /(?:^|\s)\d+\.\d+\.\d+\+([0-9a-f]{7,40})(?=\s|$)/u.exec(text);
    if (!match) throw new Error(`cannot read the host commit from --version output: ${text}`);
    return match[1];
}

/** The document the stable shell fetches first, on the published HTTPS origin. */
export function publishedManifestUrl(origin, short) {
    const base = new URL(origin);
    if (base.protocol !== 'https:') throw new Error(`published origin must be HTTPS, got ${origin}`);
    return new URL(`/wasm/${short}/manifest.json`, base).href;
}

/**
 * Fail before navigation unless the host's artifact is actually published and
 * advertises the same engine and ticket schema. `fetchImpl` is injectable so
 * the unit test can exercise 404 and schema drift without a deployed site.
 */
export async function verifyPublishedArtifact({ short, origin, fetchImpl = fetch }) {
    if (!/^[0-9a-f]{7,40}$/u.test(short)) throw new Error(`not a host commit prefix: ${short}`);
    const url = publishedManifestUrl(origin, short);
    let response;
    try {
        response = await fetchImpl(url);
    } catch (error) {
        throw new Error(BOOT_ARTIFACT_MISSING(short), { cause: error });
    }
    if (!response.ok) throw new Error(BOOT_ARTIFACT_MISSING(short));
    let manifest;
    try {
        manifest = await response.json();
    } catch (error) {
        throw new Error(`${url} is not a JSON manifest`, { cause: error });
    }
    if (manifest?.short !== short || typeof manifest.commit !== 'string' || !manifest.commit.startsWith(short)) {
        throw new Error(`${url} names commit ${manifest?.commit ?? 'none'}, not host ${short}`);
    }
    if (manifest.ticketSchema !== 3) {
        throw new Error(`${url} has ticketSchema ${manifest.ticketSchema}, not 3; the published runtime was built without multiplayer`);
    }
    return { url, manifest };
}
