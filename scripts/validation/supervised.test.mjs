import assert from 'node:assert/strict';
import test from 'node:test';
import { command, requireSupervisor, stop, SupervisionUnavailable, unitName } from './supervised.mjs';

test('a unit name identifies one child and is systemd-safe', () => {
    const first = unitName('browser/H35/1');
    assert.equal(first, unitName('browser/H35/1'));
    assert.notEqual(first, unitName('browser/H35/2'));
    assert.match(first, /^rh-e2e-[A-Za-z0-9_.-]+$/u);
});

test('the browser child is launched with a TTL and control-group kill', () => {
    const argv = command(['google-chrome', '--headless'], {
        unit: 'rh-e2e-browser', budgetS: 900, env: { ROBIN_X: '1' },
    });
    assert.deepEqual(argv.slice(0, 2), ['systemd-run', '--user']);
    assert.ok(argv.includes('--property=RuntimeMaxSec=900s'));
    assert.ok(argv.includes('--property=KillMode=control-group'));
    assert.ok(argv.includes('--setenv=ROBIN_X=1'));
    assert.deepEqual(argv.slice(-3), ['--', 'google-chrome', '--headless']);
});

test('a missing systemd prerequisite fails before spawning', () => {
    assert.throws(() => requireSupervisor(() => null), SupervisionUnavailable);
});

test('stop escalates only when needed and reports both failures', () => {
    const seen = [];
    const failures = stop('rh-e2e-browser', argv => {
        seen.push(argv);
        return { ok: false, error: 'Unit not found' };
    });
    assert.equal(seen.length, 2);
    assert.equal(failures.length, 2);
    assert.deepEqual(seen[1], ['systemctl', '--user', 'kill', '-s', 'SIGKILL', 'rh-e2e-browser']);
    const once = stop('rh-e2e-browser', () => ({ ok: true, error: '' }));
    assert.deepEqual(once, []);
});
