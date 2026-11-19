// Diagnostic acceptance only: production bundles + current local wasm + read-only Demo data.
// Uses installed Chrome/CDP, not a mocked engine. All hostname resolution is loopback.
import { createServer } from 'node:https';
import { spawn, execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync, appendFileSync, mkdtempSync, mkdirSync, readdirSync, statSync, existsSync, rmSync } from 'node:fs';
import { resolve, join, extname, sep } from 'node:path';
import { tmpdir } from 'node:os';
import { createHash } from 'node:crypto';
import { parseArgs } from 'node:util';

const { values } = parseArgs({ options: {
    data: { type: 'string' }, pkg: { type: 'string', default: 'wasm-www/pkg' },
    mission: { type: 'string' },
    evidence: { type: 'string' }, 'probe-only': { type: 'boolean' }, isolated: { type: 'boolean' },
    signer: { type: 'boolean' },
    'natural-play': { type: 'boolean' },
    'replay-file': { type: 'string' },
    'bind-fetch-diagnostic': { type: 'boolean' }, 'runtime-source': { type: 'string', default: 'HEAD' },
    'join-ticket': { type: 'string' }, 'host-http': { type: 'string', default: 'http://127.0.0.1:18641' },
    'join-content': { type: 'string' }, 'join-datadir': { type: 'string' }, 'join-modules': { type: 'string' },
    'join-log-timeout': { type: 'string', default: '300000' },
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
const gameHost = 'robinhood.phiresky.xyz', signerHost = 'identity.robinhood.phiresky.xyz';
const gameOrigin = `https://${gameHost}`, signerOrigin = `https://${signerHost}`;
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
// `--data` holds the published layout of the current Demo generation directory.
const demoPrefix = '/datadirs/demo-leicester/v18/';
const demoDatadirName = 'v18-web-opus-q80.rhdata.zst';
// Join mode drives a native host's signed `rhmp3` invitation through the real
// stable shell: the page origin is still the production hostname (mapped to a
// loopback HTTPS server), but only the game and signer hostnames are redirected.
// The ticket's relay host must resolve and reach the internet normally.
const joinMode = values['join-ticket'] !== undefined;
const hostHttp = values['host-http'];
const joinLogTimeout = Number(values['join-log-timeout']);
const joinContent = values['join-content'] ? resolve(values['join-content']) : null;
const joinDatadir = values['join-datadir'] ? resolve(values['join-datadir']) : null;
// A Demo generation converted here is served from this acceptance origin; the
// published deployment serves its own generation from `/datadirs/demo-leicester/`.
const joinDemoPrefix = '/datadirs/demo-local/';
const joinModules = resolve(values['join-modules'] ?? join(pkg, 'js-modules.json'));
const runtimeContract = JSON.parse(readFileSync('wasm-www/runtime-contract.json', 'utf8'));
// The manifest this run serves, built from the artifact and the converter's own
// content manifest before navigation (see buildJoinManifest).
let joinManifest = null, joinTicket = null, relayHost = null;
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
const cert = join(evidence, 'localhost-cert.pem'), key = join(evidence, 'localhost-key.pem');
execFileSync('openssl', ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1', '-keyout', key,
    '-out', cert, '-subj', '/CN=robinhood.phiresky.xyz', '-addext', `subjectAltName=DNS:${gameHost},DNS:${signerHost}`], { stdio: 'ignore' });
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
    if (values.isolated || joinMode) {
        output['Cross-Origin-Opener-Policy'] = 'same-origin';
        output['Cross-Origin-Embedder-Policy'] = 'require-corp';
        output['Cross-Origin-Resource-Policy'] = 'cross-origin';
    }
    return output;
}
const mime = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.json': 'application/json',
    '.css': 'text/css', '.png': 'image/png', '.ttf': 'font/ttf', '.opus': 'audio/ogg', '.zst': 'application/zstd' };
const server = createServer({ cert: readFileSync(cert), key: readFileSync(key) }, (req, res) => {
    const host = req.headers.host?.split(':')[0];
    const url = new URL(req.url, gameOrigin);
    log('requests.jsonl', { host, method: req.method, path: url.pathname });
    if (req.method !== 'GET' || ![gameHost, signerHost].includes(host)) { res.writeHead(403); res.end('local acceptance: denied'); return; }
    const kind = host === signerHost ? 'signer' : 'public';
    for (const [name, value] of Object.entries(headers(kind, url.pathname))) res.setHeader(name, value);
    if (url.pathname === '/acceptance-away') { res.setHeader('Content-Type', 'text/html'); res.end('<title>Local BFCache destination</title>'); return; }
    if (joinMode) {
        // The manifest is this run's statement of which engine bytes and which
        // content generation the ticket's invitation selects, derived from the
        // staged artifact and from `convert_datadir --web-content-manifest`.
        // It is served from memory so it can never disagree with the files the
        // handler below hands out.
        if (url.pathname === `/wasm/${short}/manifest.json`) {
            const bytes = Buffer.from(`${JSON.stringify(joinManifest, null, 2)}\n`);
            res.setHeader('Content-Type', 'application/json');
            res.setHeader('Content-Length', bytes.length);
            res.end(bytes);
            return;
        }
        // Staged runtime addition: `<root>/wasm/<short>/…` plus
        // `<root>/wasm/latest.json`, so the ticket-selected path is served by
        // the same layout production uses. The signer origin serves its own
        // shell, exactly as the two deployed origins do.
        let root = resolve(kind === 'signer' ? 'wasm-www/signer-dist' : 'wasm-www/dist');
        let path = decodeURIComponent(url.pathname);
        if (path.startsWith('/wasm/')) root = pkg;
        else if (path.startsWith(joinDemoPrefix) && joinContent) {
            root = joinContent; path = path.slice(joinDemoPrefix.length - 1);
        }
        if (path.endsWith('/')) path += 'index.html';
        const file = resolve(root, `.${path}`);
        if (!file.startsWith(root + sep) || !existsSync(file) || !statSync(file).isFile()) { res.writeHead(404); res.end('not found'); return; }
        res.setHeader('Content-Type', mime[extname(file)] ?? (/\.zst$/u.test(file) ? 'application/zstd' : 'application/octet-stream'));
        const bytes = readFileSync(file); res.setHeader('Content-Length', bytes.length); res.end(bytes); return;
    }
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
const port = server.address().port;
const profile = mkdtempSync(join(evidence, 'chrome-profile-'));
let chrome, socket, runtimeException; let id = 0; const pending = new Map(); const events = [];
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
async function waitForBrowserRunning(fromIndex, label) {
    const started = Date.now();
    for (;;) {
        const state = await browserStateOrNull();
        if (state?.multiplayer?.admission === 'running' && state.frame > 0) return state;
        if (Date.now() - started > joinLogTimeout) {
            throw new Error(`${label}: browser never reached a running multiplayer mission; last state ${JSON.stringify(state)}`);
        }
        await pause(1000);
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
async function hostEventsSince(sequence) { return eventPage(await hostJson(`/events?since=${sequence}&limit=200`)); }
async function waitForHostEvent(kind, predicate = () => true, timeoutMs = 60_000) {
    const started = Date.now();
    const first = await hostJson('/state');
    const since = first.observability?.next_event_seq ?? 0;
    for (;;) {
        const page = await hostEventsSince(since);
        const match = eventPage(page).find(event => event.kind === kind && predicate(event));
        if (match !== undefined) return match;
        if (Date.now() - started > timeoutMs) throw new Error(`host never recorded event ${kind}`);
        await pause(500);
    }
}
async function attachTarget() {
    const target = await cdp('Target.createTarget', { url: 'about:blank' });
    const attached = (await cdp('Target.attachToTarget', { targetId: target.targetId, flatten: true })).sessionId;
    for (const domain of ['Page', 'Runtime', 'Network']) await cdp(`${domain}.enable`, {}, attached);
    await cdp('Fetch.enable', { patterns: [{ urlPattern: '*' }] }, attached);
    await cdp('Page.addScriptToEvaluateOnNewDocument', { source: `
        window.__acceptanceOriginalFetch = window.fetch;
        if (window.AudioContext) { const Original = AudioContext;
          window.AudioContext = class extends Original { constructor(...args) { super(...args); } };
        }
    ` }, attached);
    return { target, sessionId: attached };
}
// The manifest a production stage publishes (`stageRuntimeAddition`) binds the
// published Demo generation, which a checkout here cannot reproduce. Join mode
// therefore authors the same document for the generation it actually serves,
// from that generation's own `robinhood-web-content.json` — the converter's
// statement of identity, datadir digest and every shipping file — and every
// digest in it is re-measured from the bytes this server hands out.
function buildJoinManifest() {
    if (joinContent === null || joinDatadir === null) {
        throw new Error('join mode needs --join-content and --join-datadir');
    }
    const webPath = join(joinContent, 'robinhood-web-content.json');
    const web = JSON.parse(readFileSync(webPath, 'utf8'));
    if (web.schema !== runtimeContract.contentSchema) {
        throw new Error(`Demo content manifest schema ${web.schema} is not the runtime contract's ${runtimeContract.contentSchema}`);
    }
    if (web.edition !== joinTicket.content_edition) {
        throw new Error(`Demo content edition ${web.edition} is not the invitation's ${joinTicket.content_edition}`);
    }
    if (web.engine_version !== joinTicket.engine_version) {
        throw new Error(`Demo content generation was converted by engine ${web.engine_version}, not the invitation's ${joinTicket.engine_version}`);
    }
    if (web.native_content_sha256 !== joinTicket.content_identity_sha256) {
        throw new Error('Demo content identity does not match the host-signed invitation');
    }
    const datadir = readFileSync(joinDatadir);
    if (digest(datadir) !== web.datadir.sha256 || datadir.length !== web.datadir.byte_length) {
        throw new Error(`served Demo datadir is not the object ${webPath} names`);
    }
    if (joinDatadir !== resolve(joinContent, web.datadir.path)) {
        throw new Error(`--join-datadir is not ${web.datadir.path} under --join-content`);
    }
    for (const file of web.files) {
        const bytes = readFileSync(resolve(joinContent, file.path));
        if (digest(bytes) !== file.sha256 || bytes.length !== file.byte_length) {
            throw new Error(`served Demo shipping file ${file.path} is not the object ${webPath} names`);
        }
    }
    const claims = JSON.parse(readFileSync(joinModules, 'utf8'));
    const artifactRoot = resolve(pkg, 'wasm', joinTicket.engine_version.slice(0, 12));
    for (const claim of claims) {
        const bytes = readFileSync(resolve(artifactRoot, claim.path));
        if (digest(bytes) !== claim.sha256 || bytes.length !== claim.byteLength) {
            throw new Error(`staged runtime module ${claim.path} is not the module ${joinModules} claims`);
        }
    }
    const manifest = {
        commit: joinTicket.engine_version, short: joinTicket.engine_version.slice(0, 12),
        netProtocol: joinTicket.net_protocol, ticketSchema: joinTicket.schema,
        multiplayerContent: {
            schema: runtimeContract.contentSchema,
            demo: {
                url: `${gameOrigin}${joinDemoPrefix}${web.datadir.path}`,
                sha256: web.datadir.sha256, byteLength: web.datadir.byte_length,
                nativeContentSha256: web.native_content_sha256,
            },
            full: null,
        },
        javascriptModules: claims,
    };
    if (!existsSync(resolve(pkg, 'wasm', manifest.short, 'robin.js'))) {
        throw new Error(`--pkg must be the staging root whose wasm/${manifest.short}/ holds the engine`);
    }
    return manifest;
}
async function runJoinFlow() {
    record('joinTicket', {
        engineVersion: joinTicket.engine_version, artifactShort: joinTicket.engine_version.slice(0, 12),
        schema: joinTicket.schema, netProtocol: joinTicket.net_protocol, missionId: joinTicket.mission_id,
        expectedPlayers: joinTicket.expected_players, contentEdition: joinTicket.content_edition,
        relayHost, contentIdentity: joinTicket.content_identity_sha256,
    });
    joinManifest = buildJoinManifest();
    const demo = joinManifest.multiplayerContent.demo;
    record('joinManifest', {
        commit: joinManifest.commit, short: joinManifest.short, ticketSchema: joinManifest.ticketSchema,
        demo, javascriptModules: joinManifest.javascriptModules.length,
    });
    const joinUrl = `${gameOrigin}/?wasm-log=debug#join=${values['join-ticket']}`;
    console.log('NAVIGATE', `${gameOrigin}/?wasm-log=debug#join=<redacted-ticket>`);
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
    const browserState = await waitForBrowserRunning(journalStart, 'initial join');
    record('browserState', browserState);
    const hostState = await waitForHostCondition('admitted a running session', state =>
        state.multiplayer?.admission === 'running' && state.frame > 0);
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
    const seatEvent = await waitForHostEvent('seat_connected', event => event.seat === browserState.multiplayer.local_seat);
    record('hostSeatEvent', seatEvent);
    const joinFrame = Math.max(hostState.frame, browserState.frame);
    // The host compares canonical per-tick state hashes periodically. A zero
    // desync count after at least one comparison beyond the join frame is the
    // objective world-agreement assertion; a screenshot is only supporting.
    const compare = await waitForHostCondition(
        `compared a canonical state hash past frame ${joinFrame} without a desync`,
        state => state.multiplayer?.desync_count === 0
            && state.multiplayer?.last_compared_hash_frame !== null
            && state.multiplayer.last_compared_hash_frame > joinFrame,
        180_000,
    );
    const settledBrowser = await browserStateOrNull();
    record('hashAgreement', {
        hostLastCompared: compare.multiplayer.last_compared_hash_frame,
        hostDesyncCount: compare.multiplayer.desync_count,
        browserFrame: settledBrowser.frame, hostFrame: compare.frame,
        browserDesyncCount: settledBrowser.multiplayer.desync_count,
        browserLastCompared: settledBrowser.multiplayer.last_compared_hash_frame,
    });
    if (settledBrowser.multiplayer.desync_count !== 0) throw new Error('browser reported a desync');
    const shot = await cdp('Page.captureScreenshot', { format: 'png' }, session);
    writeFileSync(join(evidence, 'join.png'), Buffer.from(shot.data, 'base64'));
    // ── leave: the browser closes without saying goodbye; the host must not hang.
    const frameAtLeave = (await browserStateOrNull()).frame;
    const leaveState = await hostJson('/state');
    const leaveSince = leaveState.observability?.next_event_seq ?? 0;
    await cdp('Target.closeTarget', { targetId: target.targetId });
    session = undefined;
    const disconnect = await (async () => {
        const started = Date.now();
        for (;;) {
            const page = await hostEventsSince(leaveSince);
            const match = eventPage(page).find(event => event.kind === 'transport_disconnected');
            if (match !== undefined) return match;
            if (Date.now() - started > 120_000) throw new Error('host never observed the browser transport loss');
            await pause(500);
        }
    })();
    record('hostDisconnectEvent', disconnect);
    const afterLeave = await waitForHostCondition(
        `kept running after the browser left frame ${frameAtLeave}`,
        state => state.frame > frameAtLeave, 120_000,
    );
    record('hostAfterLeave', { frame: afterLeave.frame, admission: afterLeave.multiplayer?.admission });
    // ── rejoin: a fresh page, the same invitation and durable seat key.
    const reopened = await attachTarget();
    session = reopened.sessionId;
    journalStart = events.length;
    await cdp('Page.navigate', { url: joinUrl }, session);
    await waitForConsole('browser multiplayer client welcomed', joinLogTimeout, journalStart);
    const rejoinedBrowser = await waitForBrowserRunning(journalStart, 'rejoin');
    const rejoinedHost = await waitForHostCondition('served the rejoined seat', state =>
        state.multiplayer?.admission === 'running' && state.frame > afterLeave.frame);
    record('rejoin', {
        browserSeat: rejoinedBrowser.multiplayer.local_seat, hostSeat: rejoinedHost.multiplayer.local_seat,
        browserFrame: rejoinedBrowser.frame, hostFrame: rejoinedHost.frame,
        beginSimFrame: rejoinedBrowser.multiplayer.begin_sim_frame,
        map: rejoinedBrowser.map,
    });
    if (rejoinedBrowser.multiplayer.local_seat !== browserState.multiplayer.local_seat) {
        throw new Error('rejoin did not reclaim the parked seat');
    }
    if (rejoinedBrowser.multiplayer.begin_sim_frame !== browserState.multiplayer.begin_sim_frame) {
        throw new Error('rejoin changed the begin-sim boundary');
    }
    const rejoinCompare = await waitForHostCondition(
        'compared a canonical state hash after the rejoin',
        state => state.multiplayer?.desync_count === 0
            && state.multiplayer?.last_compared_hash_frame > rejoinedBrowser.multiplayer.begin_sim_frame,
        180_000,
    );
    record('rejoinHashAgreement', {
        hostLastCompared: rejoinCompare.multiplayer.last_compared_hash_frame,
        hostDesyncCount: rejoinCompare.multiplayer.desync_count,
        hostFrame: rejoinCompare.frame,
    });
    const rejoinShot = await cdp('Page.captureScreenshot', { format: 'png' }, session);
    writeFileSync(join(evidence, 'rejoin.png'), Buffer.from(rejoinShot.data, 'base64'));
    record('joinProof', {
        missionId: joinTicket.mission_id, map: browserState.map,
        beginSimFrame: browserState.multiplayer.begin_sim_frame,
        browserSeat: browserState.multiplayer.local_seat,
        firstComparisonFrame: compare.multiplayer.last_compared_hash_frame,
        secondComparisonFrame: rejoinCompare.multiplayer.last_compared_hash_frame,
        desyncs: 0,
    });
}

const deadline = Date.now() + Number(values.timeout);const timer = setTimeout(() => chrome?.kill('SIGKILL'), Number(values.timeout) + 10000);
try {
    chrome = spawn('google-chrome', ['--headless=new', '--no-first-run', '--no-default-browser-check', '--disable-background-networking',
        '--no-proxy-server', '--ignore-certificate-errors', '--remote-debugging-port=0', `--user-data-dir=${profile}`,
        '--enable-unsafe-webgpu', '--enable-unsafe-swiftshader', '--use-angle=swiftshader',
        `--host-resolver-rules=${joinMode
            ? `MAP ${gameHost} 127.0.0.1:${port}, MAP ${signerHost} 127.0.0.1:${port}`
            : `MAP * 127.0.0.1:${port}`}`, '--window-size=1280,900', 'about:blank'], { stdio: ['ignore', 'ignore', 'pipe'] });
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
    const target = await cdp('Target.createTarget', { url: 'about:blank' });
    session = (await cdp('Target.attachToTarget', { targetId: target.targetId, flatten: true })).sessionId;
    for (const domain of ['Page', 'Runtime', 'Network']) await cdp(`${domain}.enable`, {}, session);
    await cdp('Fetch.enable', { patterns: [{ urlPattern: '*' }] }, session);
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
        await runJoinFlow();
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
        if (!values['probe-only']) {
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
    clearTimeout(timer); socket?.close(); chrome?.kill('SIGTERM');
    if (chrome && chrome.exitCode === null) await Promise.race([new Promise(resolve => chrome.once('exit', resolve)), pause(3000)]);
    if (chrome && chrome.exitCode === null) chrome.kill('SIGKILL');
    server.closeAllConnections(); await new Promise(resolve => server.close(resolve));
    for (const waiter of pending.values()) waiter.reject(new Error('harness closed')); pending.clear();
    writeFileSync(join(evidence, 'result.json'), JSON.stringify(result, null, 2));
    if (result.failures.length > 0) process.exitCode = 1;
    rmSync(profile, { recursive: true, force: true });
    rmSync(key, { force: true });
    console.log(`Evidence: ${evidence}`);
}
