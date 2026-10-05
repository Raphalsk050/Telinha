'use strict';

const assert = require('node:assert/strict');
const { test } = require('node:test');

const {
  compareVersions, decide, parseNotes, parseVersion,
} = require('../scripts/release');

function notesText({
  features = ['None.'], improvements = ['None.'], fixes = ['None.'], notes = '',
} = {}) {
  const section = (items) => items.map((item, index) => (item === 'None.' ? item : `${index + 1}. ${item}`));
  return [
    '# Telinha release notes',
    '## Features', '', ...section(features), '',
    '## Improvements', '', ...section(improvements), '',
    '## Fixes', '', ...section(fixes),
    ...(notes ? ['', 'Notes:', notes] : []),
    '',
  ].join('\n');
}

test('parseNotes reads the three sections and the optional notes', () => {
  const notes = parseNotes(notesText({
    features: ['First thing', 'Second thing'], fixes: ['A fix'], notes: 'Read this.',
  }));
  assert.deepEqual(notes.features, ['First thing', 'Second thing']);
  assert.deepEqual(notes.improvements, []);
  assert.deepEqual(notes.fixes, ['A fix']);
  assert.equal(notes.notes, 'Read this.');
});

test('parseNotes accepts CRLF and trailing spaces on headings', () => {
  const text = notesText({ fixes: ['A fix'] }).replace('## Improvements', '## Improvements ').replace(/\n/g, '\r\n');
  assert.deepEqual(parseNotes(text).fixes, ['A fix']);
});

test('parseNotes refuses notes outside the template', () => {
  const valid = notesText({ fixes: ['A fix'] });
  assert.throws(() => parseNotes(notesText()), /nenhum item/);
  assert.throws(() => parseNotes(valid.replace('# Telinha release notes', '# Notes')), /comecam/);
  assert.throws(() => parseNotes(valid.replace('## Improvements', '## Changes')), /falta a secao/);
  assert.throws(() => parseNotes(valid.replace('1. A fix', '2. A fix')), /numerado/);
  assert.throws(() => parseNotes(valid.replace('1. A fix', '- A fix')), /numerado/);
  assert.throws(() => parseNotes(`${valid}\nStray text\n`), /numerado/);
  assert.throws(() => parseNotes(`${valid}\n## Other\n\n1. More\n`), /depois de Fixes/);
  assert.throws(() => parseNotes(`${valid}\nNotes:\n`), /depois de Fixes/);
});

test('decide publishes only a version above the last release', () => {
  assert.deepEqual(decide({ version: [0, 1, 0], latest: null, notesReleased: false }),
    { version: '0.1.0', release: true, waiting: false });
  assert.deepEqual(decide({ version: [0, 2, 0], latest: [0, 1, 0], notesReleased: false }),
    { version: '0.2.0', release: true, waiting: false });
  assert.deepEqual(decide({ version: [0, 1, 0], latest: [0, 1, 0], notesReleased: true }),
    { version: '0.1.0', release: false, waiting: false });
});

test('decide waits when the notes moved on but the version did not', () => {
  assert.deepEqual(decide({ version: [0, 1, 0], latest: [0, 1, 0], notesReleased: false }),
    { version: '0.1.0', release: false, waiting: true });
});

test('decide refuses a version going back or a release repeating the old notes', () => {
  assert.throws(() => decide({ version: [0, 1, 0], latest: [0, 2, 0], notesReleased: false }), /menor/);
  assert.throws(() => decide({ version: [0, 3, 0], latest: [0, 2, 0], notesReleased: true }), /notas ainda/);
});

test('versions parse and compare numerically', () => {
  assert.deepEqual(parseVersion('v0.10.2'), [0, 10, 2]);
  assert.equal(parseVersion('0.10'), null);
  assert.ok(compareVersions([0, 10, 0], [0, 9, 9]) > 0);
  assert.equal(compareVersions([1, 2, 3], [1, 2, 3]), 0);
});
