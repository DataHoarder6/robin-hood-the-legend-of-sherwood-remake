// Pure contract tests for the issue #76 browser reload check. These run in the
// `tooling` gate suite without a browser: only digests, counts, kinds, byte
// sizes, map names and frames are used — no name, save payload or credential.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
    PERSISTENCE_ENDPOINT,
    assertPersistenceSurvivesReload,
    assertCorruptionReported,
    assertLiveMissionAfterReload,
} from './campaign-persistence.mjs';

const profile = (overrides = {}) => ({
    present: true,
    identityDigest: 'id',
    configDigest: 'cfg',
    attemptCount: 2,
    errorKind: null,
    archiveBytes: 100,
    ...overrides,
});
const resume = (overrides = {}) => ({
    available: true, missionId: 7, slotKind: 'continue', errorKind: null, ...overrides,
});
const snapshot = (p = {}, r = {}) => ({ profile: profile(p), resume: resume(r) });

test('the probe endpoint has the name the browser driver calls', () => {
    assert.equal(PERSISTENCE_ENDPOINT, 'campaign-persistence');
});

test('stable identity/config, retained attempts and a resume target pass', () => {
    const proof = assertPersistenceSurvivesReload(snapshot(), snapshot({ attemptCount: 3 }));
    assert.deepEqual(proof, {
        identityStable: true, configStable: true, attemptsBefore: 2, attemptsAfter: 3,
        resumeAvailable: true, resumeMissionId: 7, archiveBytesAfter: 100,
    });
});

test('a missing or null snapshot field fails closed', () => {
    assert.throws(() => assertPersistenceSurvivesReload({}, snapshot()), /missing profile \(pre-reload\)/);
    assert.throws(() => assertPersistenceSurvivesReload(snapshot(), {}), /missing profile \(post-reload\)/);
    assert.throws(() => assertPersistenceSurvivesReload({ profile: profile() }, snapshot()), /missing resume \(pre-reload\)/);
    assert.throws(() => assertPersistenceSurvivesReload(snapshot(), { profile: profile() }), /missing resume \(post-reload\)/);
    const withoutErrorKind = profile();
    delete withoutErrorKind.errorKind;
    assert.throws(() => assertPersistenceSurvivesReload(
        { profile: withoutErrorKind, resume: resume() }, snapshot(),
    ), /profile.errorKind is missing/);
    assert.throws(() => assertPersistenceSurvivesReload(
        { profile: profile(), resume: {} }, snapshot(),
    ), /resume.available is missing/);
});

test('a rejected archive, changed identity/config or lost progress fails', () => {
    assert.throws(() => assertPersistenceSurvivesReload(snapshot({ errorKind: 'corrupt' }), snapshot()), /already reported corrupt/);
    assert.throws(() => assertPersistenceSurvivesReload(snapshot(), snapshot({ errorKind: 'corrupt' })), /post-reload profile reported corrupt/);
    assert.throws(() => assertPersistenceSurvivesReload(snapshot({ present: false }), snapshot()), /archive is absent/);
    assert.throws(() => assertPersistenceSurvivesReload(snapshot({ identityDigest: 'a' }), snapshot({ identityDigest: 'b' })), /identity changed/);
    assert.throws(() => assertPersistenceSurvivesReload(snapshot({ configDigest: 'a' }), snapshot({ configDigest: 'b' })), /config changed/);
    assert.throws(() => assertPersistenceSurvivesReload(snapshot({ attemptCount: 3 }), snapshot({ attemptCount: 2 })), /truncated/);
    assert.throws(() => assertPersistenceSurvivesReload(snapshot({ attemptCount: 0 }), snapshot({ attemptCount: 0 })), /no campaign attempt/);
    assert.throws(() => assertPersistenceSurvivesReload(snapshot(), snapshot({}, { available: false })), /no Continue\/resume target/);
});

test('corrupt and oversize archives are reported explicitly', () => {
    assert.deepEqual(assertCorruptionReported({ profile: { errorKind: 'corrupt', archiveBytes: 10 } }, 'corrupt'),
        { errorKind: 'corrupt', archiveBytes: 10 });
    assert.deepEqual(assertCorruptionReported({ profile: { errorKind: 'oversize', archiveBytes: 4194305 } }, 'oversize'),
        { errorKind: 'oversize', archiveBytes: 4194305 });
    assert.throws(() => assertCorruptionReported({ profile: { errorKind: 'corrupt' } }, 'oversize'), /expected oversize/);
    assert.throws(() => assertCorruptionReported({ profile: {} }, 'corrupt'), /did not report a profile errorKind/);
});

test('a reload without a live mission fails: durable bytes alone are not resume', () => {
    // The shipped minipc evidence recorded map:null/frame:null after reload and
    // still passed because the guard was `savedState?.map && reloadedState?.map`.
    assert.throws(() => assertLiveMissionAfterReload({ map: null, frame: null }), /no live engine/);
    assert.throws(() => assertLiveMissionAfterReload(null), /no live engine/);
    assert.throws(() => assertLiveMissionAfterReload({ map: '', frame: 0 }), /no live engine/);
    assert.throws(() => assertLiveMissionAfterReload({ map: 'leicester', frame: null }), /no live engine/);
    assert.throws(() => assertLiveMissionAfterReload({ map: 'leicester', frame: Number.NaN }), /no live engine/);
    assert.deepEqual(assertLiveMissionAfterReload({ map: 'leicester', frame: 0 }), { map: 'leicester', frame: 0 });
});
