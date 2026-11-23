// Pure assertions for the issue #76 browser reload check.
//
// These helpers consume only the non-sensitive snapshot produced by the
// runtime's `campaign-persistence` RPC: digests, counts, kinds and byte sizes.
// No profile name, save payload, credential or private-origin password is ever
// read by this module, recorded, or returned in a failure message.

/** Fields the runtime must expose for the reload contract to be provable. */
export const PERSISTENCE_ENDPOINT = 'campaign-persistence';

/**
 * Assert the profile and campaign state survived a real page reload.
 *
 * `before` and `after` are the snapshots returned by `campaign-persistence`.
 * Both are plain objects; absent fields are treated as a contract failure, not
 * as `undefined === undefined`. Returns the small, safe summary the caller
 * records in `--evidence`.
 */
export function assertPersistenceSurvivesReload(before, after) {
    const required = ['profile', 'resume'];
    for (const key of required) {
        if (!before || typeof before[key] !== 'object' || before[key] === null) {
            throw new Error(`campaign-persistence snapshot is missing ${key} (pre-reload)`);
        }
        if (!after || typeof after[key] !== 'object' || after[key] === null) {
            throw new Error(`campaign-persistence snapshot is missing ${key} (post-reload)`);
        }
    }
    for (const [where, snapshot] of [['pre-reload', before], ['post-reload', after]]) {
        for (const field of ['identityDigest', 'configDigest', 'attemptCount', 'errorKind']) {
            if (!Object.hasOwn(snapshot.profile, field)) {
                throw new Error(`campaign-persistence profile.${field} is missing (${where})`);
            }
        }
        if (!Object.hasOwn(snapshot.resume, 'available')) {
            throw new Error(`campaign-persistence resume.available is missing (${where})`);
        }
    }
    if (before.profile.errorKind !== null) {
        throw new Error(`pre-reload profile already reported ${before.profile.errorKind}`);
    }
    if (after.profile.errorKind !== null) {
        throw new Error(`post-reload profile reported ${after.profile.errorKind}`);
    }
    if (!before.profile.present || !after.profile.present) {
        throw new Error('player profile archive is absent');
    }
    if (before.profile.identityDigest !== after.profile.identityDigest) {
        throw new Error('player profile identity changed across reload');
    }
    if (before.profile.configDigest !== after.profile.configDigest) {
        throw new Error('player profile config changed across reload');
    }
    if (after.profile.attemptCount < before.profile.attemptCount) {
        throw new Error('campaign attempt history was truncated across reload');
    }
    if (before.profile.attemptCount === 0) {
        throw new Error('no campaign attempt was persisted before reload');
    }
    if (!after.resume.available) {
        throw new Error('no Continue/resume target is available after reload');
    }
    return {
        identityStable: true,
        configStable: true,
        attemptsBefore: before.profile.attemptCount,
        attemptsAfter: after.profile.attemptCount,
        resumeAvailable: after.resume.available,
        resumeMissionId: after.resume.missionId ?? null,
        archiveBytesAfter: after.profile.archiveBytes ?? null,
    };
}

/**
 * Assert the engine has a live mission after the reload. The durable probe is
 * answered off the mission queue, so it can succeed while the page is still at
 * the main menu; digest/count stability alone would then prove only that bytes
 * survived, not that boot auto-resume put the player back in the campaign.
 * Only the non-sensitive map name and frame are read.
 */
export function assertLiveMissionAfterReload(state) {
    if (!state || typeof state !== 'object') {
        throw new Error('no live engine state after reload');
    }
    if (!Number.isFinite(state.frame)) {
        throw new Error('no live engine frame after reload');
    }
    if (typeof state.map !== 'string' || state.map.length === 0) {
        throw new Error('no live engine mission map after reload');
    }
    return { map: state.map, frame: state.frame };
}

/**
 * Assert a corrupted / oversized archive is reported explicitly and is never
 * silently replaced. The runtime reports a stable `errorKind`, not raw bytes.
 */
export function assertCorruptionReported(snapshot, expectedKind) {
    if (!snapshot?.profile || !Object.hasOwn(snapshot.profile, 'errorKind')) {
        throw new Error('campaign-persistence did not report a profile errorKind');
    }
    if (snapshot.profile.errorKind !== expectedKind) {
        throw new Error(`expected ${expectedKind}, got ${snapshot.profile.errorKind ?? 'none'}`);
    }
    // A rejected archive carries no trustworthy history count by design: the
    // probe reports errorKind and byte size, never a synthesized count. The
    // explicit errorKind — not a preserved count — is the contract here.
    return { errorKind: snapshot.profile.errorKind, archiveBytes: snapshot.profile.archiveBytes ?? null };
}
