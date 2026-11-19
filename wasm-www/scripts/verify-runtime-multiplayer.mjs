#!/usr/bin/env node
// Prove a built browser runtime's multiplayer contract without launching a
// browser: instantiate the artifact's own glue, call the export the stable
// shell calls first, and report what it says.
//
//   node wasm-www/scripts/verify-runtime-multiplayer.mjs --pkg wasm-www/pkg \
//       --expect-ticket-schema 3
//
// A runtime built without the `multiplayer` feature has no relay client and
// reports ticket schema 0; a runtime built with it reports the schema the
// published manifest names. Both are checked here, so the difference is a
// command's exit status rather than a claim in a document.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { parseArgs } from 'node:util';

const CANONICAL_KEYS = ['engineCommit', 'artifactShort', 'netProtocol', 'ticketSchema'];

// A threaded runtime's glue statically imports wasm-bindgen-rayon's
// bundler-free worker helper, which registers a `message` listener on the
// worker global as soon as the module loads. Node has no worker global, and
// nothing here starts the decode pool, so a listener stub is enough to import
// the glue; the compatibility export is the same in both thread modes.
function stubWorkerGlobal() {
    globalThis.self ??= Object.freeze({
        addEventListener() {},
        removeEventListener() {},
        postMessage() {},
    });
}

/** Instantiate `pkgDir`'s runtime and return its multiplayer compatibility. */
export async function runtimeMultiplayerCompatibility(pkgDir) {
    stubWorkerGlobal();
    const glue = resolve(pkgDir, 'robin.js');
    const wasm = resolve(pkgDir, 'robin_bg.wasm');
    const module = await import(pathToFileURL(glue).href);
    if (typeof module.initSync !== 'function' || typeof module.wasm_multiplayer_compatibility !== 'function') {
        throw new Error(`${glue} does not export initSync and wasm_multiplayer_compatibility; it is not a browser runtime`);
    }
    module.initSync({ module: await readFile(wasm) });
    const compatibility = module.wasm_multiplayer_compatibility();
    assert.equal(typeof compatibility, 'object', 'wasm_multiplayer_compatibility returned no object');
    assert.notEqual(compatibility, null);
    assert.deepEqual(Object.keys(compatibility), CANONICAL_KEYS,
        'browser runtime returned non-canonical multiplayer compatibility data');
    assert.match(compatibility.engineCommit, /^[0-9a-f]{12,40}$/u);
    assert.match(compatibility.artifactShort, /^[0-9a-f]{12}$/u);
    assert.equal(Number.isSafeInteger(compatibility.netProtocol), true);
    assert.equal(Number.isSafeInteger(compatibility.ticketSchema), true);
    return compatibility;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
    const { values } = parseArgs({ options: {
        pkg: { type: 'string', default: 'wasm-www/pkg' },
        'expect-ticket-schema': { type: 'string' },
    } });
    const expected = Number(values['expect-ticket-schema']);
    if (!Number.isSafeInteger(expected) || expected < 0) {
        throw new Error('--expect-ticket-schema must name a non-negative integer');
    }
    const compatibility = await runtimeMultiplayerCompatibility(resolve(values.pkg));
    console.log(JSON.stringify(compatibility));
    if (compatibility.ticketSchema !== expected) {
        throw new Error(`runtime ${values.pkg} exports ticketSchema ${compatibility.ticketSchema}, expected ${expected}`);
    }
    console.log(`runtime multiplayer compatibility: ticketSchema ${compatibility.ticketSchema} as required`);
}
