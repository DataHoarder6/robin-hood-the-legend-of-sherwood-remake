import assert from 'node:assert/strict';
import test from 'node:test';
import { BOOT_ARTIFACT_MISSING, hostShortFromVersion, publishedManifestUrl, verifyPublishedArtifact } from './browser-boot.mjs';

const short = '95ab7b9dbdf2';
const origin = 'https://robin.jautis.lt';
const commit = '95ab7b9dbdf25725789086ea83fd6a78a4479e1b';
const manifest = { commit, short, netProtocol: 72, ticketSchema: 3 };
const reply = (status, body) => async () => ({
    ok: status >= 200 && status < 300,
    status,
    json: async () => body,
});

test('the host version decides the manifest path', () => {
    assert.equal(hostShortFromVersion('robin 0.2.0+95ab7b9dbdf2\n'), short);
    assert.throws(() => hostShortFromVersion('robin 0.2.0'), /cannot read the host commit/u);
    assert.equal(publishedManifestUrl(origin, short), `${origin}/wasm/${short}/manifest.json`);
    assert.throws(() => publishedManifestUrl('http://robin.jautis.lt', short), /must be HTTPS/u);
});

test('a matching published artifact passes', async () => {
    const urls = [];
    const result = await verifyPublishedArtifact({ short, origin,
        fetchImpl: async url => { urls.push(url); return reply(200, manifest)(); } });
    assert.deepEqual(urls, [`${origin}/wasm/${short}/manifest.json`]);
    assert.equal(result.manifest, manifest);
});

test('a 404 fails with the host and its staging path, not a page error', async () => {
    await assert.rejects(
        verifyPublishedArtifact({ short, origin, fetchImpl: reply(404, {}) }),
        error => error.message === BOOT_ARTIFACT_MISSING(short),
    );
    await assert.rejects(
        verifyPublishedArtifact({ short, origin, fetchImpl: async () => { throw new Error('ENOTFOUND'); } }),
        error => error.message === BOOT_ARTIFACT_MISSING(short),
    );
});

test('a manifest for another commit or the wrong ticket schema fails', async () => {
    await assert.rejects(
        verifyPublishedArtifact({ short, origin, fetchImpl: reply(200, { ...manifest, commit: 'a'.repeat(40), short: 'a'.repeat(12) }) }),
        /names commit/u,
    );
    await assert.rejects(
        verifyPublishedArtifact({ short, origin, fetchImpl: reply(200, { ...manifest, ticketSchema: 0 }) }),
        /ticketSchema 0, not 3/u,
    );
});
