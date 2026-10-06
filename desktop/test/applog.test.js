'use strict';

const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { test } = require('node:test');

const { AppLog, describeError } = require('../src/applog');

function temporaryLog(t) {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'telinha-log-'));
  t.after(() => fs.rmSync(directory, { recursive: true, force: true }));
  return new AppLog(path.join(directory, 'logs', 'telinha.log'));
}

test('cada registro vira uma linha com hora e nivel, criando a pasta', (t) => {
  const log = temporaryLog(t);
  log.info('abrindo');
  log.error('caiu');

  const lines = fs.readFileSync(log.file, 'utf8').trimEnd().split('\n');
  assert.equal(lines.length, 2);
  assert.match(lines[0], /^\d{4}-\d{2}-\d{2}T[\d:.]+Z info abrindo$/);
  assert.match(lines[1], /Z erro caiu$/);
});

test('texto de varias linhas fica recuado embaixo da primeira', (t) => {
  const log = temporaryLog(t);
  log.error('falhou\r\n  em a.js:1\nem b.js:2');

  const lines = fs.readFileSync(log.file, 'utf8').trimEnd().split('\n');
  assert.match(lines[0], /Z erro falhou$/);
  assert.deepEqual(lines.slice(1), ['      em a.js:1', '    em b.js:2']);
});

test('passando do limite fica so a parte mais nova, cortada em linha inteira', (t) => {
  const log = temporaryLog(t);
  fs.mkdirSync(path.dirname(log.file), { recursive: true });
  const old = Array.from({ length: 12000 }, (_item, index) => `linha antiga ${String(index).padStart(5, '0')} ${'x'.repeat(40)}`);
  fs.writeFileSync(log.file, `${old.join('\n')}\n`);
  const before = fs.statSync(log.file).size;

  log.info('linha nova');

  const lines = fs.readFileSync(log.file, 'utf8').trimEnd().split('\n');
  assert.ok(fs.statSync(log.file).size < before * 0.6);
  assert.match(lines[0], /^linha antiga \d{5} x{40}$/);
  assert.equal(lines.at(-2), old.at(-1));
  assert.match(lines.at(-1), /Z info linha nova$/);
});

test('sem conseguir gravar o app segue em frente', (t) => {
  const log = temporaryLog(t);
  fs.mkdirSync(log.file, { recursive: true });
  assert.doesNotThrow(() => log.info('nada'));
});

test('describeError prefere a pilha e aceita o que nao e erro', () => {
  assert.match(describeError(new Error('quebrou')), /^Error: quebrou\n\s+at /);
  assert.equal(describeError('texto solto'), 'texto solto');
  assert.equal(describeError(undefined), 'undefined');
});
