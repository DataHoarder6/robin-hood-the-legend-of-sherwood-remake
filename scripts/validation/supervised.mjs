// OS-supervised child processes for the browser harness.
//
// A `detached` Chrome survives the driver that spawned it: its parent becomes
// init and the renderers keep running. This module gives each child a transient
// systemd unit with a hard TTL and a control-group kill. The driver stops the
// unit explicitly on the happy path; the TTL is the backstop when the driver
// itself is cancelled.

import { createHash } from 'node:crypto';
import { execFileSync, spawnSync } from 'node:child_process';

export class SupervisionUnavailable extends Error {}

export function requireSupervisor(which = name => {
    const result = spawnSync('sh', ['-c', `command -v ${name}`], { encoding: 'utf8' });
    return result.status === 0 ? result.stdout.trim() : null;
}) {
    const missing = ['systemd-run', 'systemctl'].filter(name => !which(name));
    if (missing.length) throw new SupervisionUnavailable(
        `process supervision requires ${missing.join(', ')} on PATH; an unsupervised browser can outlive its driver`);
}

export function unitName(scope) {
    const readable = String(scope).replace(/[^A-Za-z0-9_.-]+/gu, '-').replace(/^[.-]+|[.-]+$/gu, '').slice(0, 42) || 'child';
    const digest = createHash('sha256').update(String(scope)).digest('hex').slice(0, 12);
    return `rh-e2e-${readable}-${digest}`;
}

export function command(argv, { unit, budgetS, env = process.env }) {
    if (!Number.isInteger(budgetS) || budgetS <= 0) throw new Error('budgetS must be a positive integer');
    const wrapped = ['systemd-run', '--user',
        '--unit', unit,
        `--property=RuntimeMaxSec=${budgetS}s`,
        '--property=KillMode=control-group',
        '--property=TimeoutStopSec=10s',
        '--collect', '--wait', '--pipe', '--same-dir'];
    for (const [key, value] of Object.entries(env).sort(([a], [b]) => a.localeCompare(b))) {
        if (value !== undefined) wrapped.push(`--setenv=${key}=${value}`);
    }
    return [...wrapped, '--', ...argv];
}

export function stop(unit, run = (argv) => {
    try {
        const result = spawnSync(argv[0], argv.slice(1), { encoding: 'utf8', timeout: 10000 });
        return { ok: result.status === 0, error: result.status === 0 ? '' : (result.stderr || '').trim() };
    } catch (error) {
        return { ok: false, error: String(error) };
    }
}) {
    const failures = [];
    for (const argv of [
        ['systemctl', '--user', 'stop', unit],
        ['systemctl', '--user', 'kill', '-s', 'SIGKILL', unit],
    ]) {
        const result = run(argv);
        if (result.ok) return failures;
        if (result.error) failures.push(`${argv.join(' ')}: ${result.error}`);
    }
    return failures;
}

export function stopSync(unit) {
    try {
        execFileSync('systemctl', ['--user', 'stop', unit], { timeout: 10000, stdio: 'ignore' });
    } catch { /* the unit already exited; stop is best-effort */ }
}
