// Diagnostic acceptance only: production bundles + current local wasm + read-only Demo data.
// Uses installed Chrome/CDP, not a mocked engine. Join mode drives the published origin; the rest maps loopback.
import { createServer } from 'node:https';
import { spawn, execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync, appendFileSync, mkdtempSync, mkdirSync, readdirSync, statSync, existsSync, rmSync } from 'node:fs';
import { resolve, join, extname, sep } from 'node:path';
import { tmpdir } from 'node:os';
import { createHash } from 'node:crypto';
import { parseArgs } from 'node:util';
import { hostShortFromVersion, verifyPublishedArtifact } from './browser-boot.mjs';
import { requireSupervisor, spawnSupervised, stop as stopSupervised, unitName } from './supervised.mjs';
import { PERSISTENCE_ENDPOINT, assertPersistenceSurvivesReload, assertCorruptionReported, assertLiveMissionAfterReload,
    oversizeInjectionExpression, requireRecordedProof } from './campaign-persistence.mjs';

const { values } = parseArgs({ options: {
    data: { type: 'string' }, pkg: { type: 'string', default: 'wasm-www/pkg' },
    mission: { type: 'string' },
    evidence: { type: 'string' }, 'probe-only': { type: 'boolean' }, isolated: { type: 'boolean' },
    signer: { type: 'boolean' },
    'natural-play': { type: 'boolean' },
    // Issue #76: full-campaign reload persistence. Navigates the real artifact,
    // reaches a checkpoint, reloads the same origin and asserts the profile and
    // campaign state are retained. Only digests/counts are recorded (no names,
    // save payloads, credentials or private-origin passwords).
    'campaign-persistence': { type: 'boolean' },
    'replay-file': { type: 'string' },
    'bind-fetch-diagnostic': { type: 'boolean' }, 'runtime-source': { type: 'string', default: 'HEAD' },
    'join-ticket': { type: 'string' }, 'host-http': { type: 'string', default: 'http://127.0.0.1:18641' },
    'host-display': { type: 'string' },
    'host-binary': { type: 'string' },
    'browser-url': { type: 'string' },
    // Every wait here must outlive the game's own deadlines: a shared story page
    // holds the simulation until each seat answers it, and a seat that never
    // answers releases only when the host's acknowledgement budget expires
    // (MODAL_ACK_BUDGET_MS, 300 s — modal_session.rs). A shorter wait reads that
    // budget as a hang.
    'join-log-timeout': { type: 'string', default: '420000' },
    'cdp-timeout': { type: 'string', default: '120000' },
    timeout: { type: 'string', default: '900000' },
} });
const evidence = values.evidence ? resolve(values.evidence) : mkdtempSync(join(tmpdir(), 'robin-browser-validation-'));
mkdirSync(evidence, { recursive: true });
const pkg = resolve(values.pkg), data = values.data ? resolve(values.data) : null;
const source = execFileSync('git', ['rev-parse', values['runtime-source']], { encoding: 'utf8' }).trim();
const short = source.slice(0, 12);
const cdpTimeout = Number(values['cdp-timeout']);
if (!Number.isFinite(cdpTimeout) || cdpTimeout <= 0) throw new Error('--cdp-timeout must be a positive number of milliseconds');
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
// `--data` holds the published layout of the current Demo generation directory.
const demoPrefix = '/datadirs/demo-leicester/v18/';
const demoDatadirName = 'v18-web-opus-q80.rhdata.zst';
// Join mode drives a native host's signed `rhmp3` invitation through the real
// stable shell on its published origin. No loopback fixture or hostname
// mapping: the host's invitation URL and the artifact it selects must both be
// the ones a real browser would fetch. The relay host resolves normally.
const joinMode = values['join-ticket'] !== undefined;
if (joinMode) {
    if (values['host-display'] === undefined) {
        throw new Error('join mode needs --host-display: the host seat has nobody at its window to answer a shared story page');
    }
    if (values['host-binary'] === undefined) {
        throw new Error('join mode needs --host-binary: the host --version decides which browser artifact the shell fetches');
    }
    if (values['browser-url'] === undefined) {
        throw new Error('join mode needs --browser-url: the published HTTPS origin the browser boots from');
    }
}
// The host binary, not the checkout or the ticket, is the authority on which
// artifact the shell will request. Its --version carries the 12-hex identity
// used in /wasm/<short>/ and in every replay and invitation.
const hostVersion = joinMode ? execFileSync(values['host-binary'], ['--version'], { encoding: 'utf8' }).trim() : null;
const hostShort = joinMode ? hostShortFromVersion(hostVersion) : null;
const gameHost = joinMode ? new URL(values['browser-url']).hostname : 'robinhood.phiresky.xyz';
const gameOrigin = joinMode ? new URL(values['browser-url']).origin : `https://${gameHost}`;
const signerHost = joinMode ? null : 'identity.robinhood.phiresky.xyz';
const signerOrigin = joinMode ? null : `https://${signerHost}`;
const hostHttp = values['host-http'];
const joinLogTimeout = Number(values['join-log-timeout']);
let joinTicket = null, relayHost = null;
if (joinMode) {
    const code = values['join-ticket'];
    const parts = code.slice('rhmp3-'.length).split('.');
    if (parts.length !== 2) throw new Error('--join-ticket is not a canonical rhmp3 envelope');
    joinTicket = JSON.parse(Buffer.from(parts[0], 'base64url').toString('utf8'));
    relayHost = new URL(joinTicket.relay_url).hostname;
    if (!/^[0-9a-f]{7,40}$/.test(joinTicket.engine_version)) throw new Error('ticket engine version is not a git hash');
}
const result = { source, chrome: execFileSync('google-chrome', ['--version'], { encoding: 'utf8' }).trim(),
    shellCheckout: execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(),
    runtime: pkg, data, softwareGpuRequested: true, isolationHeadersAdded: !!values.isolated,
    diagnosticBoundFetch: !!values['bind-fetch-diagnostic'], cdpTimeoutMs: cdpTimeout, checks: {}, failures: [] };
function record(name, value) { result.checks[name] = value; console.log(name, JSON.stringify(value).slice(0, 1800)); }
function log(name, value) { appendFileSync(join(evidence, name), JSON.stringify(value) + '\n'); }
// Join mode drives the published HTTPS origin directly: there is no fixture
// server, no generated certificate and no hostname mapping to hide a missing
// artifact behind. Non-join acceptance still uses the loopback shell.
let server = null, port = null, key = null;
if (!joinMode) {
    const cert = join(evidence, 'localhost-cert.pem');
    key = join(evidence, 'localhost-key.pem');
    execFileSync('openssl', ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1', '-keyout', key,
        '-out', cert, '-subj', `/CN=${gameHost}`, '-addext', `subjectAltName=DNS:${gameHost},DNS:${signerHost}`], { stdio: 'ignore' });
    const preload = ['Data/AudioDurations.json', 'Data/Interface/Fonts/arial.ttf', ...readdirSync('assets/core-datadir/Data/Interface/UI')
        .filter(name => name.endsWith('.png')).map(name => `Data/Interface/UI/${name}`)];
    function headers(kind, path) {
        const output = {}; let active = false;
        for (const line of readFileSync(`wasm-www/deploy/${kind}-headers.txt`, 'utf8').split('\n')) {
            if (line.startsWith('/')) active = path.startsWith(line.trim().replace(/\*$/u, ''));
            else if (active && line.trim().startsWith('!')) delete output[line.trim().slice(2)];
            else if (active && line.includes(':')) {
                const index = line.indexOf(':'); output[line.slice(0, index).trim()] = line.slice(index + 1).trim();
            }
        }
        if (values.isolated) {
            output['Cross-Origin-Opener-Policy'] = 'same-origin';
            output['Cross-Origin-Embedder-Policy'] = 'require-corp';
            output['Cross-Origin-Resource-Policy'] = 'cross-origin';
        }
        return output;
    }
    const mime = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.json': 'application/json',
        '.css': 'text/css', '.png': 'image/png', '.ttf': 'font/ttf', '.opus': 'audio/ogg', '.zst': 'application/zstd' };
    server = createServer({ cert: readFileSync(cert), key: readFileSync(key) }, (req, res) => {
        const host = req.headers.host?.split(':')[0];
        const url = new URL(req.url, gameOrigin);
        log('requests.jsonl', { host, method: req.method, path: url.pathname });
        if (req.method !== 'GET' || ![gameHost, signerHost].includes(host)) { res.writeHead(403); res.end('local acceptance: denied'); return; }
        const kind = host === signerHost ? 'signer' : 'public';
        for (const [name, value] of Object.entries(headers(kind, url.pathname))) res.setHeader(name, value);
        if (url.pathname === '/acceptance-away') { res.setHeader('Content-Type', 'text/html'); res.end('<title>Local BFCache destination</title>'); return; }
        if (url.pathname === '/wasm/latest.json') { res.setHeader('Content-Type', 'application/json'); res.end(JSON.stringify({ commit: source, short })); return; }
        if (url.pathname === `/wasm/${short}/manifest.json` && data) {
            // The shell loads the Demo datadir generation pinned by the build manifest.
            const demo = readFileSync(join(data, demoDatadirName));
            res.setHeader('Content-Type', 'application/json');
            res.end(JSON.stringify({ commit: source, short, multiplayerContent: { demo: {
                url: `${gameOrigin}${demoPrefix}${demoDatadirName}`, sha256: digest(demo), byteLength: demo.length,
            } } })); return;
        }
        if (url.pathname === `/wasm/${short}/preload-assets.json`) {
            res.setHeader('Content-Type', 'application/json');
            res.end(JSON.stringify(preload.map(path => ({ path, url: `/acceptance-core/${path}` })))); return;
        }
        let root = resolve(kind === 'signer' ? 'wasm-www/signer-dist' : 'wasm-www/dist');
        let path = decodeURIComponent(url.pathname);
        if (path.startsWith('/acceptance-core/')) { root = resolve('assets/core-datadir'); path = path.slice('/acceptance-core'.length); }
        else if (path.startsWith(`/wasm/${short}/`)) { root = pkg; path = path.slice(`/wasm/${short}`.length); }
        else if (path.startsWith(demoPrefix) && data) { root = data; path = path.slice(demoPrefix.length - 1); }
        if (path.endsWith('/')) path += 'index.html';
        const file = resolve(root, `.${path}`);
        if (!file.startsWith(root + sep) || path.includes('browser_identity_vault') && kind !== 'signer'
            || !existsSync(file) || !statSync(file).isFile()) { res.writeHead(404); res.end('not found'); return; }
        res.setHeader('Content-Type', mime[extname(file)] ?? 'application/octet-stream');
        const bytes = readFileSync(file); res.setHeader('Content-Length', bytes.length); res.end(bytes);
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    port = server.address().port;
}
const profile = mkdtempSync(join(evidence, 'chrome-profile-'));
let chrome, chromeUnit, socket, runtimeException, target; let id = 0; const pending = new Map(); const events = [];
function cdp(method, params = {}, sessionId) {
    const requestId = ++id;
    return new Promise((resolve, reject) => {
        const timeout = setTimeout(() => { pending.delete(requestId); reject(new Error(`CDP timeout after ${cdpTimeout}ms: ${method}`)); }, cdpTimeout);
        pending.set(requestId, { resolve: value => { clearTimeout(timeout); resolve(value); }, reject: e => { clearTimeout(timeout); reject(e); } });
        socket.send(JSON.stringify({ id: requestId, method, params, ...(sessionId ? { sessionId } : {}) }));
    });
}
let session;
const evaluate = async expression => {
    if (runtimeException) throw runtimeException;
    const reply = await cdp('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true, userGesture: true }, session);
    if (reply.exceptionDetails) throw new Error(JSON.stringify(reply.exceptionDetails));
    return reply.result.value;
};
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
// A signal to Chrome's main process leaves its renderers, GPU and zygote
// processes running, and they keep writing to the profile directory. The
// browser is spawned as a process-group leader so teardown stops everything it
// started rather than racing the last profile flush.
async function stopChrome() {
    if (!chrome || chrome.exitCode !== null) return;
    // The unit owns Chrome's whole cgroup; killing only the wrapper's group
    // leaves renderers behind when the driver itself is killed. Stop the unit
    // first, then reap the wrapper for a clean exit event.
    if (chromeUnit) stopSupervised(chromeUnit);
    const exited = new Promise(resolve => chrome.once('exit', resolve));
    const stopped = ms => Promise.race([exited.then(() => true), pause(ms).then(() => false)]);
    for (const signal of ['SIGTERM', 'SIGKILL']) {
        try { process.kill(-chrome.pid, signal); } catch { /* the group is already gone */ }
        if (await stopped(5000)) return;
    }
}
const pressKey = async (key, code, windowsVirtualKeyCode) => {
    const down = cdp('Input.dispatchKeyEvent', { type: 'keyDown', key, code, windowsVirtualKeyCode }, session);
    // Gameplay hotkeys sample held state; a down/up pair in one frame can be
    // invisible there even though modal dialogs consume individual key events.
    // Release on a host timer without waiting for the potentially slow ack.
    await pause(500);
    await Promise.all([down, cdp('Input.dispatchKeyEvent', { type: 'keyUp', key, code, windowsVirtualKeyCode }, session)]);
};
// ── join mode ────────────────────────────────────────────────────────────
// A browser seat joins a native host through the host's signed invitation.
// Every assertion below reads the two live surfaces — the browser's own `state`
// RPC and the host's loopback `/state` + `/events` — so the proof is that both
// sides agree on one session, not that a page loaded.
const consoleText = event => event.params.args.map(arg => arg.value ?? arg.description ?? '').join(' ');
function consoleLines(fromIndex) {
    return events.slice(fromIndex).filter(e => e.method === 'Runtime.consoleAPICalled').map(consoleText);
}
async function waitForConsole(fragment, timeoutMs, fromIndex) {
    const started = Date.now();
    for (;;) {
        const line = consoleLines(fromIndex).find(candidate => candidate.includes(fragment));
        if (line !== undefined) return line;
        if (Date.now() - started > timeoutMs) {
            const tail = consoleLines(fromIndex).slice(-12).join('\n');
            throw new Error(`timed out waiting for browser console line: ${fragment}\nrecent console:\n${tail}`);
        }
        await pause(200);
    }
}
async function hostJson(path, init) {
    const response = await fetch(`${hostHttp}${path}`, init);
    if (!response.ok) throw new Error(`host ${path} returned HTTP ${response.status}: ${await response.text()}`);
    return response.json();
}
async function browserStateOrNull() {
    try { return await evaluate(`robinRpc('state')`); } catch (error) {
        if (/robinRpc is not a function|not defined|Cannot find context|execution context/u.test(error.message)) return null;
        throw error;
    }
}
async function waitForBrowserRunning(label) {
    const started = Date.now();
    // What the browser's own state RPC said, so a timeout names the surface it
    // read instead of only the last value.
    const samples = [];
    for (;;) {
        const state = await browserStateOrNull();
        const elapsed = Date.now() - started;
        const sample = { atMs: elapsed, frame: state?.frame ?? null,
            admission: state?.multiplayer?.admission ?? null,
            lastCompared: state?.multiplayer?.last_compared_hash_frame ?? null };
        log('join-state-polls.jsonl', { label, ...sample });
        samples.push(sample);
        if (state?.multiplayer?.admission === 'running') {
            record('statePolls', { label, samples });
            return state;
        }
        if (elapsed > joinLogTimeout) {
            record('statePolls', { label, samples });
            throw new Error(`${label}: browser never reached a running multiplayer mission; last state ${JSON.stringify(state)}`);
        }
        await pause(1000);
    }
}
// The host is the only seat that publishes canonical state hashes
// (`send_state_hash` is server-side only); a guest compares them with its own
// retained pre-tick states. The browser's `last_compared_hash_frame` — not the
// host's, which stays null for that reason — is therefore the parity surface,
// and a non-zero `desync_count` is the failure. `frame` is the Original's
// hourglass tick, which need not advance at all in a short mission, so it
// cannot stand in for simulation progress.
async function waitForBrowserParity(label, beyondFrame) {
    const started = Date.now();
    const samples = [];
    for (;;) {
        const state = await browserStateOrNull();
        const parity = {
            atMs: Date.now() - started,
            admission: state?.multiplayer?.admission ?? null,
            lastCompared: state?.multiplayer?.last_compared_hash_frame ?? null,
            desyncCount: state?.multiplayer?.desync_count ?? null,
        };
        log('join-parity-polls.jsonl', { label, ...parity });
        samples.push(parity);
        if (parity.lastCompared !== null && parity.lastCompared > beyondFrame) {
            record('parityPolls', { label, samples });
            if (parity.desyncCount !== 0) {
                throw new Error(`${label}: browser reported ${parity.desyncCount} desyncs`);
            }
            // `last_compared_hash_frame` also advances when a due host hash has
            // no retained local state, so a comparison that *matched* is only
            // proved by the absence of the engine's own missed-comparison line.
            const missed = consoleLines(0).filter(line => line.includes('hash comparison missed'));
            if (missed.length > 0) {
                throw new Error(`${label}: ${missed.length} host hashes were compared against no retained local state`);
            }
            return parity;
        }
        if (parity.atMs > joinLogTimeout) {
            record('parityPolls', { label, samples });
            throw new Error(`${label}: browser never compared a host hash past frame ${beyondFrame}; last state ${JSON.stringify(state)}`);
        }
        await pause(500);
    }
}
async function waitForHostCondition(describe, predicate, timeoutMs = 120_000) {
    const started = Date.now();
    for (;;) {
        const state = await hostJson('/state');
        if (predicate(state)) return state;
        if (Date.now() - started > timeoutMs) throw new Error(`host never ${describe}; last state ${JSON.stringify(state)}`);
        await pause(500);
    }
}
function eventPage(page) { return Array.isArray(page.events) ? page.events : []; }
// `/events` pages at 200; a session that records a burst between two polls
// must not be read as if the rest never happened.
async function hostEventsSince(sequence) {
    const events = [];
    for (;;) {
        const page = await hostJson(`/events?since=${sequence}&limit=200`);
        const batch = eventPage(page);
        events.push(...batch);
        if (batch.length < 200 || typeof page.next_seq !== 'number' || page.next_seq <= sequence) return events;
        sequence = page.next_seq;
    }
}
// ── answering shared story pages ─────────────────────────────────────────
// A shared page (mission briefing, `DisplayPopupText` scroll) holds the
// simulation until the seats in the session answer it. The host with nobody at
// its window is such a seat: a silent host holds the frame until its own
// acknowledgement budget expires (MODAL_ACK_BUDGET_MS, 300 s), and a browser
// seat that joins before that expiry never gets past `peer_waiting_for_begin`.
//
// The host records each announcement in its event ring (`modal_announced`,
// `role: "host"`); the warden answers it with the same synthetic Return the
// native harness uses on a peer's display (`scripts/validation/briefing_x11.py`).
// The browser seat does not answer pages of its own: a peer's copy is dismissed
// by the host's decision (`modal_acked`, "modal decided (host authority)"), and
// the browser's ring carries no `modal_announced` at all.
//
// Each announcement is one page instance, so the ring's own `seq` keys what
// was answered: a page shown twice is two instances, and a seat that skipped
// one would hold the session again.
const answeredPages = new Set();
let pageWardenStopped = false;
async function pageWarden() {
    const hostDisplay = values['host-display'];
    // From the ring's first event: the host may announce a page during boot,
    // before this driver attaches, and that page still holds the session.
    let hostSequence = 0;
    while (!pageWardenStopped) {
        try {
            for (const event of await hostEventsSince(hostSequence)) {
                if (typeof event.seq === 'number') hostSequence = Math.max(hostSequence, event.seq + 1);
                if (event.kind !== 'modal_announced' || event.detail?.role !== 'host') continue;
                if (answeredPages.has(event.seq)) continue;
                answeredPages.add(event.seq);
                execFileSync('python3', ['scripts/validation/briefing_x11.py', hostDisplay], { stdio: 'ignore' });
                record('hostPageAnswered', { page: String(event.detail?.modal), seq: event.seq });
            }
        } catch (error) {
            // The host is between pages or restarting its session; the next
            // poll reads its ring again. The evidence keeps the reason.
            log('page-warden-errors.jsonl', { error: error.message });
        }
        await pause(1000);
    }
}
async function waitForHostEvent(kind, predicate = () => true, timeoutMs = 60_000, since) {
    const started = Date.now();
    if (since === undefined) since = (await hostJson('/state')).observability?.next_event_seq ?? 0;
    for (;;) {
        const page = await hostEventsSince(since);
        const match = page.find(event => event.kind === kind && predicate(event));
        if (match !== undefined) return match;
        if (Date.now() - started > timeoutMs) {
            throw new Error(`host never recorded event ${kind}; ring since ${since}: ${JSON.stringify(await hostEventsSince(since))}`);
        }
        await pause(500);
    }
}
async function runJoinFlow() {
    // The host's --version, not the ticket and not this checkout, decides which
    // artifact the shell will fetch. Check the published document on the real
    // origin before the page is given a ticket to install.
    const published = await verifyPublishedArtifact({ short: hostShort, origin: gameOrigin });
    record('hostVersion', { version: hostVersion, short: hostShort });
    record('publishedArtifact', {
        url: published.url, commit: published.manifest.commit, short: published.manifest.short,
        netProtocol: published.manifest.netProtocol, ticketSchema: published.manifest.ticketSchema,
        demo: published.manifest.multiplayerContent?.demo ?? null,
        javascriptModules: published.manifest.javascriptModules?.length ?? null,
    });
    record('joinTicket', {
        engineVersion: joinTicket.engine_version, artifactShort: joinTicket.engine_version.slice(0, 12),
        schema: joinTicket.schema, netProtocol: joinTicket.net_protocol, missionId: joinTicket.mission_id,
        expectedPlayers: joinTicket.expected_players, contentEdition: joinTicket.content_edition,
        relayHost, contentIdentity: joinTicket.content_identity_sha256,
    });
    if (joinTicket.engine_version.slice(0, 12) !== hostShort) {
        throw new Error(`host ${hostShort} signed a ticket for engine ${joinTicket.engine_version.slice(0, 12)}`);
    }
    const joinUrl = `${gameOrigin}/?wasm-log=debug#join=${values['join-ticket']}`;
    console.log('NAVIGATE', `${gameOrigin}/?wasm-log=debug#join=<redacted-ticket>`);
    // From the session's own first event: the host records the peer's seat
    // before this driver can ask, so a ring read that starts at "now" misses it.
    const hostSeqAtJoin = (await hostJson('/state')).observability?.next_event_seq ?? 0;
    record('hostSeqAtJoin', hostSeqAtJoin);
    let journalStart = events.length;
    await cdp('Page.navigate', { url: joinUrl }, session);
    const welcome = await waitForConsole('browser multiplayer client welcomed', joinLogTimeout, journalStart);
    record('browserWelcomeLog', welcome);
    const invited = consoleLines(journalStart).find(line => line.includes('[authenticated browser invitation via'));
    record('browserInvitationLog', invited ?? null);
    // The shell must be cross-origin isolated: the threaded artifact imports a
    // shared memory, so COOP/COEP are part of the join contract.
    record('joinEnvironment', await evaluate(`({ secureContext: isSecureContext, crossOriginIsolated,
        origin: location.origin, hash: location.hash })`));
    const browserState = await waitForBrowserRunning('initial join');
    record('browserState', browserState);
    const hostState = await waitForHostCondition('admitted a running session', state =>
        state.multiplayer?.admission === 'running' && state.multiplayer?.begin_sim_frame !== null);
    record('hostState', hostState);
    if (browserState.multiplayer.local_seat === hostState.multiplayer.local_seat) {
        throw new Error('browser and host report the same local seat');
    }
    if (browserState.map !== hostState.map) throw new Error(`map disagreement: browser ${browserState.map} host ${hostState.map}`);
    if (browserState.multiplayer.begin_sim_frame !== hostState.multiplayer.begin_sim_frame) {
        throw new Error(`begin_sim_frame disagreement: browser ${browserState.multiplayer.begin_sim_frame} host ${hostState.multiplayer.begin_sim_frame}`);
    }
    record('worldAgreement', {
        map: browserState.map, beginSimFrame: browserState.multiplayer.begin_sim_frame,
        browserSeat: browserState.multiplayer.local_seat, hostSeat: hostState.multiplayer.local_seat,
    });
    // The host attributes a peer's connect command to the session's own seat
    // when it queues it (`seat=0` in the ring) while the engine applies it to
    // the peer's seat; the role is the fact that holds, not the number.
    const seatEvent = await waitForHostEvent('seat_connected',
        event => event.detail?.role === 'peer', 60_000, hostSeqAtJoin);
    record('hostSeatEvent', seatEvent);
    // A matched canonical hash past the begin boundary is the objective
    // world-agreement assertion; a screenshot is only supporting. 300 frames
    // is twelve of the host's 25-frame comparisons.
    const firstParity = await waitForBrowserParity('initial join', browserState.multiplayer.begin_sim_frame + 300);
    record('hashAgreement', firstParity);
    record('joinProof', {
        missionId: joinTicket.mission_id, map: browserState.map,
        beginSimFrame: browserState.multiplayer.begin_sim_frame,
        browserSeat: browserState.multiplayer.local_seat,
        comparisonFrame: firstParity.lastCompared,
        desyncs: 0,
    });
    const shot = await cdp('Page.captureScreenshot', { format: 'png' }, session);
    writeFileSync(join(evidence, 'join.png'), Buffer.from(shot.data, 'base64'));
    // ── leave: the browser closes without saying goodbye; the host must not hang.
    const leaveState = await hostJson('/state');
    const leaveSince = leaveState.observability?.next_event_seq ?? 0;
    await cdp('Target.closeTarget', { targetId: target.targetId });
    session = undefined;
    const disconnect = await (async () => {
        const started = Date.now();
        for (;;) {
            const page = await hostEventsSince(leaveSince);
            // A page that vanishes sends no goodbye: the host either evicts the
            // seat outright or records a transport loss and keeps it parked.
            const match = page.find(event => event.kind === 'seat_evicted' || event.kind === 'transport_disconnected');
            if (match !== undefined) return match;
            if (Date.now() - started > 120_000) throw new Error('host never observed the browser transport loss');
            await pause(500);
        }
    })();
    record('hostDisconnectEvent', disconnect);
    const afterLeave = await hostJson('/state');
    record('hostAfterLeave', {
        admission: afterLeave.multiplayer?.admission ?? null,
        desyncCount: afterLeave.multiplayer?.desync_count ?? null,
        presentation: afterLeave.presentation?.kind ?? null,
    });
    if (afterLeave.multiplayer?.admission !== 'running') {
        throw new Error('host left the running session when the browser page disappeared');
    }
}

const deadline = Date.now() + Number(values.timeout);const timer = setTimeout(() => { void stopChrome(); }, Number(values.timeout) + 10000);

// Issue #76 acceptance: a real page reload of the same origin keeps the player
// profile and campaign progress, and a corrupt/oversize archive is reported
// explicitly. Only the runtime's non-sensitive persistence probe is recorded.
async function runCampaignPersistence() {
    if (!data) throw new Error('--data is required for the campaign persistence check');
    const probeExpr = `robinRpc('${PERSISTENCE_ENDPOINT}')`;
    const waitForProbe = async (label) => {
        let last = null;
        while (Date.now() < deadline) {
            await pause(1500);
            try {
                const snapshot = await evaluate(`Promise.race([${probeExpr}, new Promise((_,r)=>setTimeout(()=>r(Error('probe timeout')),10000))])`);
                if (snapshot && snapshot.profile && snapshot.resume) return snapshot;
                last = snapshot;
            } catch (error) { last = { error: error.message }; }
        }
        throw new Error(`${label}: campaign-persistence RPC never became available (last: ${JSON.stringify(last)?.slice(0, 300)})`);
    };

    // The persistence RPC is answered off the mission queue, so its arrival
    // says nothing about engine readiness. The ordinary flow gates on a live
    // state instead; do the same before driving `set-paused` / `mission-result`.
    const waitForMissionReady = async (label) => {
        let last = null;
        while (Date.now() < deadline) {
            await pause(1500);
            try {
                const state = await evaluate(`Promise.race([robinRpc('state'), new Promise((_,r)=>setTimeout(()=>r(Error('state timeout')),10000))])`);
                if (state && Number.isFinite(state.frame) && typeof state.map === 'string') {
                    return state;
                }
                last = { error: 'state had no live mission frame/map' };
            } catch (error) { last = { error: error.message }; }
        }
        throw new Error(`${label}: mission never became ready before the acceptance deadline (last: ${JSON.stringify(last)?.slice(0, 300)})`);
    };

    // A failed game RPC must fail this check, not be logged and forgotten:
    // without a checkpoint the later digest assertions would only fail opaque.
    const requireGameRpc = async (label, expression) => {
        try { return await evaluate(expression); }
        catch (error) { result.failures.push(`${label}: ${error.message}`); return null; }
    };

    const query = new URLSearchParams({ 'wasm-log': 'info' });
    // The private origin boots from /campaign.json; the local fixture uses the
    // documented developer forced-launch path. Never change the guest protocol.
    if (values.mission) query.set('mission', values.mission);
    await cdp('Page.navigate', { url: `${gameOrigin}/?${query}` }, session);
    const before = await waitForProbe('first boot');
    record('persistenceBefore', before);
    const readyState = await waitForMissionReady('campaign persistence');
    record('missionReady', { started: true, frame: readyState.frame });

    // Reach a durable checkpoint through the game's own terminal flow: a
    // mission result drives the terminal autosave and profile synchronization.
    await requireGameRpc('set-paused (before step)', `robinRpc('set-paused',{paused:true})`);
    await requireGameRpc('step-forward', `robinRpc('step-forward',{n:30,auto_dismiss:true})`);
    await requireGameRpc('set-paused (after step)', `robinRpc('set-paused',{paused:false})`);
    await requireGameRpc('mission-result', `robinRpc('mission-result',{result:'won'})`);
    await pause(5000);
    const afterSave = await waitForProbe('after checkpoint');
    record('persistenceAfterSave', afterSave);
    const savedState = await evaluate(`robinRpc('state')`).catch(() => null);
    record('stateAtCheckpoint', savedState ? { map: savedState.map, frame: savedState.frame } : null);
    await cdp('Page.captureScreenshot', { format: 'png' }, session)
        .then(shot => writeFileSync(join(evidence, 'campaign-before-reload.png'), Buffer.from(shot.data, 'base64'))).catch(() => {});

    // A genuine reload of the same origin — not the bfcache history restore
    // used by the generic flow — is what issue #76 is about.
    await cdp('Page.reload', { ignoreCache: true }, session);
    const afterReload = await waitForProbe('after reload');
    record('persistenceAfterReload', afterReload);
    // The probe is answered off the queue, so it can succeed while the page is
    // still at the main menu. A reload only proves the campaign resumed when a
    // live engine state arrives; a null/absent map or frame is a hard failure,
    // not a reason to skip the mission comparison.
    const reloadedState = await waitForMissionReady('after reload')
        .then(assertLiveMissionAfterReload)
        .catch(error => { result.failures.push(`campaign reload: ${error.message}`); return null; });
    record('stateAfterReload', reloadedState);
    await cdp('Page.captureScreenshot', { format: 'png' }, session)
        .then(shot => writeFileSync(join(evidence, 'campaign-after-reload.png'), Buffer.from(shot.data, 'base64'))).catch(() => {});

    let summary = null;
    try { summary = assertPersistenceSurvivesReload(afterSave, afterReload); }
    catch (error) { result.failures.push(`campaign reload: ${error.message}`); }
    if (summary) record('campaignReloadProof', summary);
    if (reloadedState && savedState?.map && savedState.map !== reloadedState.map) {
        result.failures.push(`campaign resumed on a different mission: ${savedState.map} -> ${reloadedState.map}`);
    }

    // Corrupt archive: reported explicitly, never silently zeroed.
    await evaluate(`localStorage.setItem('robin-hood-player-profiles-v1', JSON.stringify({schema_version:999,manager:{}}))`);
    await cdp('Page.reload', { ignoreCache: true }, session);
    const corrupt = await waitForProbe('after corrupt injection');
    try { record('corruptProof', assertCorruptionReported(corrupt, 'corrupt')); }
    catch (error) { result.failures.push(`corrupt report: ${error.message}`); }

    // Oversize archive: reported explicitly. The runtime rejects on UTF-8 byte
    // length (>4 MiB, Rust `String::len`) but Chrome budgets localStorage in
    // UTF-16 code units (<5 MiB per origin), so a 3-byte BMP character crosses
    // the runtime limit inside the browser quota. A single `setItem` is atomic
    // and replaces the prior corrupt value. Any injection/reload/probe
    // exception is recorded here — it must not escape runCampaignPersistence
    // into the outer catch and silently skip this assertion.
    try {
        await evaluate(oversizeInjectionExpression());
        await cdp('Page.reload', { ignoreCache: true }, session);
        const oversize = await waitForProbe('after oversize injection');
        try { record('oversizeProof', assertCorruptionReported(oversize, 'oversize')); }
        catch (error) { result.failures.push(`oversize report: ${error.message}`); }
    } catch (error) {
        result.failures.push(`oversize injection: ${error.message}`);
    }
    // A skipped or swallowed assertion cannot pass: the proof must be present.
    try { requireRecordedProof(result.checks, 'oversizeProof'); }
    catch (error) { result.failures.push(`oversize proof: ${error.message}`); }
}

try {
    requireSupervisor();
    const chromeArgv = ['google-chrome', '--headless=new', '--no-first-run', '--no-default-browser-check', '--disable-background-networking',
        '--no-proxy-server', '--ignore-certificate-errors', '--remote-debugging-port=0', `--user-data-dir=${profile}`,
        '--enable-unsafe-webgpu', '--enable-unsafe-swiftshader', '--use-angle=swiftshader',
        // Join mode resolves the published origin and the host's relay exactly
        // as a browser on the internet would; only the fixture needs a mapping.
        ...(joinMode ? [] : [`--host-resolver-rules=MAP * 127.0.0.1:${port}`]),
        '--window-size=1280,900', 'about:blank'];
    chromeUnit = unitName(`chrome-${evidence}-${process.pid}`);
    chrome = spawnSupervised(chromeArgv, {
        unit: chromeUnit, budgetS: Math.ceil(Number(values.timeout) / 1000) + 30,
        spawnOptions: { detached: true, stdio: ['ignore', 'ignore', 'pipe'] },
    });
    const ws = await new Promise((resolve, reject) => {
        let stderr = ''; const startup = setTimeout(() => reject(new Error('Chrome debugging endpoint unavailable')), 20000);
        chrome.stderr.on('data', chunk => {
            appendFileSync(join(evidence, 'chrome.log'), chunk); stderr += chunk;
            const match = stderr.match(/DevTools listening on (ws:\/\/[^\s]+)/u);
            if (match) { clearTimeout(startup); resolve(match[1]); }
        });
        chrome.once('error', reject);
    });
    socket = new WebSocket(ws); await new Promise(resolve => socket.addEventListener('open', resolve, { once: true }));
    socket.addEventListener('message', ({ data }) => {
        const message = JSON.parse(data);
        if (message.id) {
            const waiter = pending.get(message.id); pending.delete(message.id);
            if (message.error) waiter?.reject(new Error(JSON.stringify(message.error))); else waiter?.resolve(message.result);
        } else {
            events.push(message); log('cdp-events.jsonl', message);
            if (!values['probe-only'] && message.method === 'Runtime.exceptionThrown') {
                runtimeException = new Error('Uncaught browser runtime exception; inspect cdp-events.jsonl');
                // A panicked wasm game cannot service queued RPC. Fail promptly
                // on the actual exception, not a later, misleading CDP timeout.
                for (const waiter of pending.values()) waiter.reject(runtimeException);
                pending.clear();
            }
            if (message.method === 'Fetch.requestPaused') {
                const request = message.params.request;
                const url = new URL(request.url);
                const allowed = request.method === 'GET' && url.protocol === 'https:'
                    && ([gameHost, signerHost].includes(url.hostname) || (joinMode && url.hostname === relayHost)) && !url.port;
                void cdp(allowed ? 'Fetch.continueRequest' : 'Fetch.failRequest', {
                    requestId: message.params.requestId, ...(!allowed ? { errorReason: 'BlockedByClient' } : {}),
                }, message.sessionId).catch(error => log('interception-errors.jsonl', error.message));
                if (!allowed) log('blocked-requests.jsonl', request);
            }
        }
    });
    target = await cdp('Target.createTarget', { url: 'about:blank' });
    session = (await cdp('Target.attachToTarget', { targetId: target.targetId, flatten: true })).sessionId;
    for (const domain of ['Page', 'Runtime', 'Network']) await cdp(`${domain}.enable`, {}, session);
    if (!joinMode) await cdp('Fetch.enable', { patterns: [{ urlPattern: '*' }] }, session);
    await cdp('Page.addScriptToEvaluateOnNewDocument', { source: `
        window.__acceptanceOriginalFetch = window.fetch;
        ${values['bind-fetch-diagnostic'] ? 'window.fetch = window.fetch.bind(window);' : ''}
        window.__acceptance = { contexts: [], decoded: 0, started: 0, lifecycle: [] };
        for (const event of ['pagehide', 'pageshow']) addEventListener(event, e => __acceptance.lifecycle.push({ event, persisted: e.persisted }));
        if (window.AudioContext) { const Original = AudioContext;
          window.AudioContext = class extends Original { constructor(...args) { super(...args); __acceptance.contexts.push(this); }
            decodeAudioData(...args) { __acceptance.decoded++; return super.decodeAudioData(...args); }
          };
        }
        // Rust constructs AudioBufferSourceNode directly; wrapping only
        // AudioContext.createBufferSource would miss actual playback starts.
        if (window.AudioBufferSourceNode) { const start=AudioBufferSourceNode.prototype.start;
            AudioBufferSourceNode.prototype.start=function(...args){__acceptance.started++;return start.apply(this,args);};
        }
    ` }, session);
    if (joinMode) {
        const warden = pageWarden();
        try { await runJoinFlow(); }
        finally { pageWardenStopped = true; await warden; }
    } else {
        await cdp('Page.navigate', { url: `${gameOrigin}/acceptance-away` }, session); await pause(1500);
        record('environment', await evaluate(`(async()=>{ const a = await navigator.gpu?.requestAdapter(); return {
            secureContext: isSecureContext, crossOriginIsolated, gpu: !!navigator.gpu, adapter: a ? {vendor:a.info.vendor,architecture:a.info.architecture,device:a.info.device,description:a.info.description,isFallbackAdapter:a.info.isFallbackAdapter} : null,
            webgl: !!document.createElement('canvas').getContext('webgl2'), origin: location.origin }; })()`));
        record('nativeFetchReceiver', await evaluate(`(async()=>{
            const fetcher = __acceptanceOriginalFetch;
            const detached = (await fetcher('/acceptance-away')).status;
            let objectMethod;
            try { objectMethod = (await ({ fetch: fetcher }).fetch('/acceptance-away')).status; }
            catch(error) { objectMethod = String(error); }
            return {detached,objectMethod};
        })()`));
        if (values['replay-file']) {
            const replay = JSON.parse(readFileSync(values['replay-file']));
            const workerFile = readdirSync('wasm-www/dist/assets').find(name => name.startsWith('replay_validation_worker-') && name.endsWith('.js'));
            if (!workerFile || typeof replay.content !== 'string') throw new Error('worker or compact replay missing');
            record('replayValidatorHash', digest(readFileSync(join(pkg, 'replay_admission_bg.wasm'))));
            record('replayFileHash', digest(readFileSync(values['replay-file'])));
            const reply = await evaluate(`new Promise((resolve,reject)=>{
                const worker=new Worker('/assets/${workerFile}',{type:'module'});
                const timer=setTimeout(()=>{worker.terminate();reject(Error('replay worker timeout'));},20000);
                worker.onmessage=e=>{clearTimeout(timer);worker.terminate();resolve(e.data);};
                worker.onerror=e=>{clearTimeout(timer);worker.terminate();reject(Error(e.message));};
                worker.postMessage({compact:new Uint8Array(${JSON.stringify(replay)}),
                    jsUrl:${JSON.stringify(`${gameOrigin}/wasm/${short}/replay_admission.js`)},
                    wasmUrl:${JSON.stringify(`${gameOrigin}/wasm/${short}/replay_admission_bg.wasm`)}});
            })`);
            record('isolatedReplayAdmission', reply);
            if (reply.status !== 'accepted') throw new Error('isolated replay validation rejected');
        }
        if (values.signer) {
            record('signer', await evaluate(`(async()=>{
                const origin = ${JSON.stringify(signerOrigin)};
                const frame = document.createElement('iframe');
                frame.setAttribute('sandbox','allow-scripts allow-same-origin');
                frame.src = origin + '/identity-signer/';
                const received = [];
                const ready = new Promise((resolve,reject)=>{
                    const timer=setTimeout(()=>reject(Error('signer ready timeout')),30000);
                    addEventListener('message',function listener(e){
                        if(e.origin!==origin||e.source!==frame.contentWindow)return;
                        received.push(e.data);
                        if(e.data.kind==='ready'){clearTimeout(timer);removeEventListener('message',listener);resolve();}
                    });
                });
                document.body.append(frame);await ready;
                let domAccessDenied=false;
                try { void frame.contentWindow.document.body; } catch { domAccessDenied=true; }
                const request=operation=>new Promise((resolve,reject)=>{
                    const requestId=crypto.randomUUID().replaceAll('-','');
                    const timer=setTimeout(()=>reject(Error(operation+' timeout')),10000);
                    addEventListener('message',function listener(e){
                        if(e.origin===origin&&e.source===frame.contentWindow&&(e.data.requestId===requestId
                            || operation==='export_private_key'&&e.data.error?.code==='invalid_operation')){
                            clearTimeout(timer);removeEventListener('message',listener);resolve(e.data);
                        }
                    });
                    frame.contentWindow.postMessage({protocol:'robinhood.browser-identity.v1',requestId,operation},origin);
                });
                const status=await request('status');
                const prohibited=await request('export_private_key');
                const databases=await indexedDB.databases();
                return {domAccessDenied,status,prohibited,gameOriginDatabases:databases.map(d=>d.name),sandbox:frame.getAttribute('sandbox')};
            })()`));
        }
        if (values['campaign-persistence']) {
            await runCampaignPersistence();
        }
        if (!values['probe-only'] && !values['campaign-persistence']) {
            if (!data) throw new Error('--data is required for game acceptance');
            record('runtimeHash', digest(readFileSync(join(pkg, 'robin_bg.wasm'))));
            record('dataHash', digest(readFileSync(join(data, demoDatadirName))));
            const manifest = JSON.parse(readFileSync(join(data, 'robinhood-web-content.json')));
            for (const entry of [manifest.datadir, ...manifest.files]) {
                const name = entry.path === 'datadir.bin' ? demoDatadirName : entry.path;
                const bytes = readFileSync(join(data, name));
                if (bytes.length !== entry.byte_length || digest(bytes) !== entry.sha256) throw new Error(`Demo closure mismatch: ${name}`);
            }
            record('dataClosure', { sourceEngineVersion: manifest.engine_version, schema: manifest.schema, filesVerified: manifest.files.length + 1 });
            // Ordinary Demo startup creates the canonical mission team. --mission is
            // a developer forced-launch path and must be opted into explicitly.
            const query = new URLSearchParams({ 'wasm-log': 'info' });
            if (values.mission) query.set('mission', values.mission);
            const gameEventStart = events.length;
            await cdp('Page.navigate', { url: `${gameOrigin}/?${query}` }, session);
            let state, lastProgress;
            while (Date.now() < deadline) {
                await pause(3000);
                // Startup performs synchronous wasm work and decodes hundreds of
                // audio assets. Observe already-delivered CDP console events instead
                // of requiring page evaluation while its main thread is busy.
                const gameEvents = events.slice(gameEventStart);
                const progress = gameEvents.filter(e => e.method === 'Runtime.consoleAPICalled')
                    .map(e => e.params.args.map(arg => arg.value ?? '').join(' '));
                const latestProgress = progress.slice(-2).join('\n').slice(-1800);
                if (latestProgress !== lastProgress) console.log('game-progress', latestProgress);
                lastProgress = latestProgress;
                if (gameEvents.some(e => e.method === 'Runtime.exceptionThrown')) throw new Error('Game raised an exception during startup');
                if (progress.some(line => /Recording replay/u.test(line))) {
                    try { state = await evaluate(`Promise.race([robinRpc('state'), new Promise((_,r)=>setTimeout(()=>r(Error('state timeout')),10000))])`); break; }
                    catch (error) { console.log('state not ready', error.message); }
                }
            }
            record('state', state ?? null);
            if (!state) result.failures.push('Mission never supplied a live state before the acceptance deadline.');
            record('audio', await evaluate(`(async()=>{ await Promise.all(__acceptance.contexts.map(c=>c.resume())); return { decoded:__acceptance.decoded, started:__acceptance.started, contexts:__acceptance.contexts.map(c=>({state:c.state,sampleRate:c.sampleRate})) }; })()`));
            await pause(2000);
            const screenshot = await cdp('Page.captureScreenshot', { format: 'png' }, session);
            writeFileSync(join(evidence, 'game.png'), Buffer.from(screenshot.data, 'base64'));
            record('page', await evaluate(`({title:document.title, canvas:[...document.querySelectorAll('canvas')].map(c=>({width:c.width,height:c.height})),log:document.querySelector('#log')?.textContent})`));
            if (state) {
                try {
                    if (values['natural-play']) {
                        // Return is the popup's authored confirmation binding; fixed
                        // pointer coordinates depended on one old fallback layout.
                        // Hold across a normal game frame, but queue release without
                        // waiting for acknowledgments from a slow software renderer.
                        await pressKey('Enter', 'Enter', 13);
                        await pause(5000);
                        const naturalState = await evaluate(`robinRpc('state')`);
                        record('naturalState', naturalState);
                        if (naturalState.frame <= state.frame) result.failures.push('Ordinary input did not advance the simulation beyond the initial popup frame.');
                        record('audioAfterInput', await evaluate(`({decoded:__acceptance.decoded,started:__acceptance.started,states:__acceptance.contexts.map(c=>c.state)})`));
                        const playing = await cdp('Page.captureScreenshot', { format: 'png' }, session);
                        writeFileSync(join(evidence, 'playing.png'), Buffer.from(playing.data, 'base64'));
                        // Exercise the actual pause/objectives rendering path that
                        // previously panicked on missing short-briefing text zero.
                        await pressKey('Escape', 'Escape', 27);
                        await pause(2000);
                        const pauseMenu = await cdp('Page.captureScreenshot', { format: 'png' }, session);
                        writeFileSync(join(evidence, 'pause.png'), Buffer.from(pauseMenu.data, 'base64'));
                        record('pauseMenuState', await evaluate(`robinRpc('state')`));
                        await pressKey('Escape', 'Escape', 27);
                    } else {
                        record('pause', await evaluate(`robinRpc('set-paused',{paused:true})`));
                        record('step', await evaluate(`robinRpc('step-forward',{n:5,auto_dismiss:true})`));
                    }
                    const replay = await evaluate(`robinRpc('get-replay').then(reply => Array.from(reply.data))`);
                    writeFileSync(join(evidence, 'replay.rhrec'), Buffer.from(replay));
                    record('replayExport', { type: typeof replay, length: JSON.stringify(replay).length });
                    if (Array.isArray(replay) && existsSync(join(pkg, 'replay_admission.js'))) {
                        record('replayValidatorHash', digest(readFileSync(join(pkg, 'replay_admission_bg.wasm'))));
                        const workerFile = readdirSync('wasm-www/dist/assets').find(name => name.startsWith('replay_validation_worker-') && name.endsWith('.js'));
                        if (!workerFile) throw new Error('production replay validation worker not found');
                        const reply = await evaluate(`new Promise((resolve,reject)=>{
                            const worker=new Worker('/assets/${workerFile}',{type:'module'});
                            const timer=setTimeout(()=>{worker.terminate();reject(Error('replay worker timeout'));},30000);
                            worker.onmessage=e=>{clearTimeout(timer);worker.terminate();resolve(e.data);};
                            worker.onerror=e=>{clearTimeout(timer);worker.terminate();reject(Error(e.message));};
                            worker.postMessage({compact:new Uint8Array(${JSON.stringify(replay)}),
                                jsUrl:${JSON.stringify(`${gameOrigin}/wasm/${short}/replay_admission.js`)},
                                wasmUrl:${JSON.stringify(`${gameOrigin}/wasm/${short}/replay_admission_bg.wasm`)}});
                        })`);
                        record('replayAdmission', reply);
                        if (reply.status !== 'accepted') throw new Error('recorded replay rejected by actual wasm admission worker');
                        record('replayLoad', await evaluate(`(async()=>{
                            const module=await import('/wasm/${short}/robin.js');
                            module.wasm_mark_compact_replay_validated(new Uint8Array(${JSON.stringify(replay)}));
                            return await robinRpc('load-replay',{data:new Uint8Array(${JSON.stringify(replay)}),paused:false});
                        })()`));
                        // load-replay only stages the bytes. Activate the real pause
                        // menu's Restart row (Continue, Load, Save, Options, Restart).
                        await evaluate(`robinRpc('set-paused',{paused:false})`);
                        await pressKey('Escape', 'Escape', 27);
                        for (let row = 0; row < 4; row++) await pressKey('ArrowDown', 'ArrowDown', 40);
                        const restartMenu = await cdp('Page.captureScreenshot', { format: 'png' }, session);
                        writeFileSync(join(evidence, 'replay-restart-menu.png'), Buffer.from(restartMenu.data, 'base64'));
                        await pressKey('Enter', 'Enter', 13);
                        let replayState;
                        while (Date.now() < deadline) {
                            await pause(1500);
                            replayState = await evaluate(`robinRpc('state')`);
                            if (replayState.replay && replayState.replay.frame >= replayState.replay.total) break;
                        }
                        record('replayState', replayState ?? null);
                        if (!replayState?.replay || replayState.replay.frame < replayState.replay.total) throw new Error('Staged replay never completed actual mission playback');
                        const replayScreen = await cdp('Page.captureScreenshot', { format: 'png' }, session);
                        writeFileSync(join(evidence, 'replay.png'), Buffer.from(replayScreen.data, 'base64'));
                    }
                } catch (error) { result.failures.push(`replay/control: ${error.message}`); }
            }
            if (runtimeException) throw runtimeException;
            await cdp('Page.navigate', { url: `${gameOrigin}/acceptance-away` }, session); await pause(1500);
            const history = await cdp('Page.getNavigationHistory', {}, session);
            await cdp('Page.navigateToHistoryEntry', { entryId: history.entries[history.currentIndex - 1].id }, session);
            await pause(2500);
            // A restored render frame may need a new CDP attachment even though the
            // JS document itself survived. Do not mistake that for a game crash.
            try {
                record('bfcache', await evaluate(`({ lifecycle: __acceptance.lifecycle, notRestoredReasons: performance.getEntriesByType('navigation')[0]?.notRestoredReasons?.toJSON?.() ?? null })`));
            } catch (error) {
                // A timeout is not evidence of a replaced attachment. Preserve it
                // instead of starting another long command on a wedged renderer.
                if (!/not attached|session.*not found|cannot find context|execution context was destroyed/iu.test(error.message)) throw error;
                session = (await cdp('Target.attachToTarget', { targetId: target.targetId, flatten: true })).sessionId;
                await cdp('Runtime.enable', {}, session);
                record('bfcache', await evaluate(`({ lifecycle: __acceptance.lifecycle, notRestoredReasons: performance.getEntriesByType('navigation')[0]?.notRestoredReasons?.toJSON?.() ?? null })`));
            }
            record('stateAfterHistoryRestore', await evaluate(`robinRpc('state')`));
        }
    }
    record('pageExceptions', events.filter(e => e.method === 'Runtime.exceptionThrown').map(e => e.params));
    record('networkFailures', events.filter(e => e.method === 'Network.loadingFailed').map(e => e.params));
    record('networkEndpoints', [...new Set(events.filter(e => e.method === 'Network.responseReceived')
        .map(e => e.params.response.remoteIPAddress).filter(Boolean))]);
    const consoleLines = events.filter(e => e.method === 'Runtime.consoleAPICalled')
        .map(e => e.params.args.map(arg => arg.value ?? '').join(' '));
    const contentErrors = consoleLines
        .filter(line => /level descriptors unavailable|Failed to load level descriptors|Short-briefing text table .* unavailable|required localized short briefing|DisplayPopupText\([^)]*\): text lookup failed/u.test(line));
    record('contentErrors', contentErrors);
    if (!values['probe-only'] && contentErrors.length) result.failures.push('Required mission text/descriptor loading failed.');
    // Playback logs state-hash mismatches without necessarily throwing. EOF
    // alone must not turn a divergent replay into a successful acceptance run.
    const replayErrors = consoleLines.filter(line => /Replay desync|replay save-marker desync|replay .*diverged/iu.test(line));
    record('replayErrors', replayErrors);
    if (!values['probe-only'] && replayErrors.length) result.failures.push('Replay playback reported a deterministic mismatch.');
    if (!values['probe-only'] && events.some(e => e.method === 'Runtime.exceptionThrown')) {
        result.failures.push('Game page raised uncaught exceptions; inspect pageExceptions and screenshot.');
    }
} catch (error) {
    result.failures.push(error.stack); console.error(error); process.exitCode = 1;
    if (session && socket?.readyState === WebSocket.OPEN) {
        try {
            const screenshot = await cdp('Page.captureScreenshot', { format: 'png' }, session);
            writeFileSync(join(evidence, 'failure.png'), Buffer.from(screenshot.data, 'base64'));
        } catch (captureError) { result.failures.push(`failure screenshot: ${captureError.message}`); }
    }
}
finally {
    clearTimeout(timer); socket?.close(); await stopChrome();
    if (server) { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); }
    for (const waiter of pending.values()) waiter.reject(new Error('harness closed')); pending.clear();
    writeFileSync(join(evidence, 'result.json'), JSON.stringify(result, null, 2));
    if (result.failures.length > 0) process.exitCode = 1;
    // Renderers and zygotes keep writing to the profile after the browser's main
    // process exits; stopChrome reaps the whole group, and the retry absorbs the
    // last flush instead of failing the run on a non-empty directory.
    rmSync(profile, { recursive: true, force: true, maxRetries: 10, retryDelay: 200 });
    if (key) rmSync(key, { force: true });
    console.log(`Evidence: ${evidence}`);
}
