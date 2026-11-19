import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { RUNTIME_FEATURES, runtimeBuildPlan, replayAdmissionBuildPlan } from './build-runtime.mjs';

// The section that states a browser seat can join a native host. While it is
// there, the runtime plan it describes must carry the relay client.
const MULTIPLAYER_DOC = new URL('../../docs/MULTIPLAYER.md', import.meta.url);
const BROWSER_SUPPORT_CLAIM = /^### Browser invitations, identity, and content$/mu;

function features(plan) {
    const index = plan.cargo.indexOf('--features');
    assert(index > 0, 'game plan names an explicit feature set');
    return plan.cargo[index + 1].split(',');
}

test('game and benchmark plans preserve profile, feature and output boundaries', () => {
    const plain = runtimeBuildPlan();
    assert(plain.cargo.includes('--locked'));
    assert.deepEqual(features(plain), [...RUNTIME_FEATURES]);
    assert(!plain.cargo.includes('--config'));
    assert(plain.bindgenArgs.includes('--out-name'));
    const threaded = runtimeBuildPlan({ threads: true, bench: true, profile: 'wasm-dev', optimize: false });
    assert(threaded.cargo.includes('wasm-threads'));
    assert(threaded.cargo.includes('scripts/wasm-threads.cargo-config.toml'));
    assert.equal(threaded.wasm, 'target/wasm32-unknown-unknown/wasm-dev/examples/wasm_decode_bench.wasm');
    assert(!threaded.bindgenArgs.includes('--out-name'));
    assert.equal(threaded.optimize, null);
    assert.throws(() => runtimeBuildPlan({ profile: 'release' }), /unsupported/u);
});

test('every browser runtime plan carries the relay client the docs promise', async () => {
    const doc = await readFile(MULTIPLAYER_DOC, 'utf8');
    if (!BROWSER_SUPPORT_CLAIM.test(doc)) return;
    for (const threads of [false, true]) {
        const plan = runtimeBuildPlan({ threads });
        assert.deepEqual(
            features(plan).filter(feature => feature !== 'wasm-threads'),
            [...RUNTIME_FEATURES],
            `docs/MULTIPLAYER.md states a browser seat joins, so the ${threads ? 'threaded' : 'plain'} `
                + 'runtime plan must build with multiplayer. Fix: add it to RUNTIME_FEATURES in '
                + 'wasm-www/scripts/build-runtime.mjs (a build without it exports ticket schema 0 and '
                + 'the shell refuses every invitation).',
        );
        assert.equal(features(plan).includes('multiplayer'), true);
        assert.equal(features(plan).includes('wasm-threads'), threads);
    }
});

test('replay admission has its own capped non-threaded build and never overwrites game outputs', () => {
    const plan = replayAdmissionBuildPlan({ outDir: '/tmp/test-admission' });
    assert(plan.cargo.includes('scripts/replay-admission-wasm.cargo-config.toml'));
    assert(plan.cargo.includes('robin_replay_admission_wasm'));
    assert(!plan.cargo.includes('wasm-threads'));
    assert(plan.bindgenArgs.includes('replay_admission'));
    assert.equal(plan.optimizedWasm, '/tmp/test-admission/replay_admission_bg.wasm');
});
