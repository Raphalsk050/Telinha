'use strict';

const assert = require('node:assert/strict');
const { test } = require('node:test');

const {
  CODE_PREFIX,
  DEV_EXE,
  buildShareArgs,
  buildWatchArgs,
  looksLikeCode,
  normalizeCode,
  parseEventLine,
  resolveTelinhaExe,
} = require('../src/telinha-process');

const SAMPLE_CODE = `${CODE_PREFIX}VExTMQEAAgAxCwAAdj0wDQpvPS0gMjYw`;
const WINDOW = { min: 51200, max: 51219 };
const PORTS = ['--port-min', '51200', '--port-max', '51219'];

test('parseEventLine reads one JSON event per line and ignores everything else', () => {
  assert.deepEqual(parseEventLine('{"event":"state","state":"Connected"}'), {
    event: 'state',
    state: 'Connected',
  });
  assert.equal(parseEventLine('  reservando memoria'), null);
  assert.equal(parseEventLine('{"state":"Connected"}'), null);
  assert.equal(parseEventLine('{quebrado'), null);
});

test('looksLikeCode accepts prefixed and legacy codes and ignores whitespace', () => {
  assert.equal(looksLikeCode(SAMPLE_CODE), true);
  assert.equal(looksLikeCode(`  ${SAMPLE_CODE.slice(0, 20)}\n${SAMPLE_CODE.slice(20)}  `), true);
  assert.equal(looksLikeCode(SAMPLE_CODE.slice(CODE_PREFIX.length)), true);
  assert.equal(looksLikeCode('ola, tudo bem?'), false);
  assert.equal(looksLikeCode(`${CODE_PREFIX}VExT`), false);
  assert.equal(looksLikeCode(`${SAMPLE_CODE}!`), false);
});

test('normalizeCode strips every whitespace character', () => {
  assert.equal(normalizeCode(' a b\r\nc\t'), 'abc');
  assert.equal(normalizeCode(undefined), '');
});

test('buildShareArgs maps the chosen target and audio scope', () => {
  assert.deepEqual(buildShareArgs({ targetKind: 'monitor', targetIndex: 1, audio: 'none', ports: WINDOW }), [
    'send', '--json', '--monitor', '1', '--audio', 'none', ...PORTS,
  ]);
  assert.deepEqual(
    buildShareArgs({
      targetKind: 'window', targetIndex: 3, audio: 'process', audioPid: 4242, ports: WINDOW,
    }),
    ['send', '--json', '--window', '3', '--audio', 'process', '--audio-pid', '4242', ...PORTS]);
  assert.throws(() => buildShareArgs({ targetKind: 'monitor', targetIndex: -1 }));
  assert.throws(() => buildShareArgs({ targetKind: 'window', targetIndex: 0, audio: 'process' }));
  assert.throws(() => buildShareArgs({ targetKind: 'desktop', targetIndex: 0 }));
});

test('buildShareArgs prefers the stable handle over the list index', () => {
  assert.deepEqual(
    buildShareArgs({
      targetKind: 'window', targetHandle: 0x1a2b, targetIndex: 7, audio: 'system', ports: WINDOW,
    }),
    ['send', '--json', '--window-handle', '6699', '--audio', 'system', ...PORTS]);
  assert.deepEqual(
    buildShareArgs({ targetKind: 'monitor', targetHandle: 65537, audio: 'none', ports: WINDOW }),
    ['send', '--json', '--monitor-handle', '65537', '--audio', 'none', ...PORTS]);
  assert.throws(() => buildShareArgs({ targetKind: 'monitor', targetHandle: 0 }));
});

test('buildWatchArgs adds only the requested flags and the port range', () => {
  assert.deepEqual(buildWatchArgs({ ports: WINDOW }), ['recv', '--json', ...PORTS]);
  assert.deepEqual(buildWatchArgs({ fullscreen: true, audio: false, ports: WINDOW }), [
    'recv', '--json', '--fullscreen', '--no-audio', ...PORTS,
  ]);
});

test('each stream draws its own port window away from 50000-50100', () => {
  const args = buildWatchArgs();
  const min = Number(args[args.indexOf('--port-min') + 1]);
  const max = Number(args[args.indexOf('--port-max') + 1]);
  assert.ok(min > 50100 && max === min + 19 && max <= 59999);
});

test('resolveTelinhaExe prefers the override and falls back to the development build', () => {
  assert.equal(resolveTelinhaExe({ env: { TELINHA_EXE: __filename } }), __filename);

  const fallback = resolveTelinhaExe({ env: {} });
  assert.ok(fallback === null || fallback === DEV_EXE);
});
