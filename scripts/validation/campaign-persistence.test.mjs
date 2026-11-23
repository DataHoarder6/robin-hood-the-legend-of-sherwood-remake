// Pure contract tests for the issue #76 browser reload check. These run in the
// `tooling` gate suite without a browser: only digests, counts, kinds, byte
// sizes, map names and frames are used — no name, save payload or credential.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
    PERSISTENCE_ENDPOINT,
    assertPersistenceSurvivesReload,
    assertCorruptionReported,
    assertLiveMissionAfterReload,
} from './campaign-persistence.mjs';
import * as persistence from './campaign-persistence.mjs';

// Chrome budgets localStorage in UTF-16 code units (~5 MiB per origin), while
// the runtime rejects on Rust `String::len` (UTF-8 bytes, >4 MiB). An ASCII
// payload of 4 MiB+ therefore throws QuotaExceededError before the page can be
// reloaded; only a multibyte BMP character can cross the runtime limit inside
// the browser budget. These numbers are browser/runtime facts, kept local so
// the check verifies the helper independently.
const CHROME_QUOTA_UTF16_BYTES = 5 * 1024 * 1024;
const RUNTIME_UTF8_BYTE_LIMIT = 4 * 1024 * 1024;
const driverSource = () => readFileSync(new URL('./browser-game.mjs', import.meta.url), 'utf8');

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

test('the oversize injection payload fits Chrome UTF-16 quota but breaks the runtime UTF-8 byte limit', () => {
    // Reproduce, without a browser, the exact value the driver injects under the
    // profile key. The stub enforces Chrome's UTF-16 quota and is seeded with
    // the prior corrupt value, so a QuotaExceededError or a non-replacement
    // fails the same way it would on the private origin.
    assert.equal(typeof persistence.oversizeInjectionExpression, 'function',
        'no shared oversize-injection helper for the browser driver to use');
    let stored = 'prior-corrupt-value';
    const localStorage = {
        setItem(key, value) {
            assert.equal(key, 'robin-hood-player-profiles-v1', 'oversize payload stored under the wrong profile key');
            const utf16Bytes = value.length * 2; // one UTF-16 code unit = 2 bytes
            if (utf16Bytes > CHROME_QUOTA_UTF16_BYTES) throw new Error('QuotaExceededError');
            stored = value;
        },
    };
    const expression = persistence.oversizeInjectionExpression();
    Function('localStorage', expression)(localStorage);
    assert.ok(stored !== 'prior-corrupt-value', 'the prior corrupt value was left in place');
    const utf8Bytes = Buffer.byteLength(stored, 'utf8');
    assert.ok(utf8Bytes > RUNTIME_UTF8_BYTE_LIMIT,
        `UTF-8 byte length ${utf8Bytes} does not exceed the ${RUNTIME_UTF8_BYTE_LIMIT}-byte runtime limit`);
    assert.ok(stored.length * 2 <= CHROME_QUOTA_UTF16_BYTES,
        `UTF-16 storage cost ${stored.length * 2} exceeds the ${CHROME_QUOTA_UTF16_BYTES}-byte Chrome quota`);
    // The runtime reports the UTF-8 byte count as the oversize archive size.
    assert.deepEqual(
        assertCorruptionReported({ profile: { errorKind: 'oversize', archiveBytes: utf8Bytes } }, 'oversize'),
        { errorKind: 'oversize', archiveBytes: utf8Bytes });
});

test('the browser driver cannot skip the oversize report assertion', () => {
    const source = driverSource();
    assert.ok(/oversizeInjectionExpression\(\)/u.test(source),
        'driver must build the oversize payload from the shared budget-safe helper');
    assert.ok(/requireRecordedProof\(\s*result\.checks,\s*'oversizeProof'\s*\)/u.test(source),
        'driver must require the recorded oversize proof');
    assert.ok(!/'x'\.repeat\(/u.test(source),
        'the ASCII payload that exceeds Chrome quota must be gone');
    assert.throws(() => persistence.requireRecordedProof({}, 'oversizeProof'), /not recorded/);
    assert.deepEqual(persistence.requireRecordedProof({ oversizeProof: { errorKind: 'oversize' } }, 'oversizeProof'),
        { errorKind: 'oversize' });
});
