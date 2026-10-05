'use strict';

// Diz ao CI se ha release a publicar. A versao e escolhida por gente, em version.json: aqui ela
// so e lida e comparada com o ultimo release. As notas vem de RELEASE_NOTES.md, que lista o que
// mudou desde o ultimo release.
//
//   node desktop/scripts/release.js plan

const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');

const ROOT = path.join(__dirname, '..', '..');
const NOTES_PATH = 'RELEASE_NOTES.md';
const VERSION_PATH = 'version.json';
const TITLE = '# Telinha release notes';
const SECTIONS = ['Features', 'Improvements', 'Fixes'];
const EMPTY_SECTION = 'None.';
const NOTES_LABEL = 'Notes:';
const VERSION_PATTERN = /^v?(\d+)\.(\d+)\.(\d+)$/;

function parseVersion(text) {
  const match = VERSION_PATTERN.exec(String(text ?? '').trim());
  return match ? match.slice(1).map(Number) : null;
}

function compareVersions(a, b) {
  for (let index = 0; index < 3; index += 1) {
    if (a[index] !== b[index]) {
      return a[index] - b[index];
    }
  }
  return 0;
}

function normalize(text) {
  return String(text ?? '').replace(/\r\n/g, '\n').trim();
}

function parseNotes(text) {
  const lines = normalize(text).split('\n').map((line) => line.trimEnd()).filter((line) => line !== '');
  if (lines.shift() !== TITLE) {
    throw new Error(`as notas comecam com "${TITLE}"`);
  }

  const notes = { features: [], improvements: [], fixes: [], notes: '' };
  for (const section of SECTIONS) {
    if (lines.shift() !== `## ${section}`) {
      throw new Error(`falta a secao "## ${section}", na ordem ${SECTIONS.join(', ')}`);
    }
    const items = notes[section.toLowerCase()];
    if (lines[0] === EMPTY_SECTION) {
      lines.shift();
      continue;
    }
    while (lines.length > 0 && !lines[0].startsWith('## ') && lines[0] !== NOTES_LABEL) {
      const item = /^(\d+)\. (\S.*)$/.exec(lines.shift());
      if (!item || Number(item[1]) !== items.length + 1) {
        throw new Error(`em ${section}, cada linha e um item numerado em sequencia, ou "${EMPTY_SECTION}"`);
      }
      items.push(item[2]);
    }
    if (items.length === 0) {
      throw new Error(`a secao ${section} sem itens leva "${EMPTY_SECTION}"`);
    }
  }

  if (lines.length > 0) {
    if (lines.shift() !== NOTES_LABEL || lines.length === 0) {
      throw new Error(`depois de Fixes so pode vir "${NOTES_LABEL}" seguido do texto`);
    }
    notes.notes = lines.join('\n');
  }
  if (notes.features.length + notes.improvements.length + notes.fixes.length === 0) {
    throw new Error('as notas nao tem nenhum item');
  }
  return notes;
}

// version e latest sao [a, b, c], latest e null antes do primeiro release. notesReleased diz se as
// notas sao as mesmas que sairam no ultimo release.
function decide({ version, latest, notesReleased }) {
  const label = version.join('.');
  const order = latest ? compareVersions(version, latest) : 1;
  if (order < 0) {
    throw new Error(`a versao ${label} em ${VERSION_PATH} e menor que a ultima publicada, ${latest.join('.')}`);
  }
  if (order > 0 && notesReleased) {
    throw new Error(`a versao subiu para ${label} mas as notas ainda sao as do release anterior`);
  }
  return { version: label, release: order > 0, waiting: order === 0 && !notesReleased };
}

function git(...args) {
  return execFileSync('git', args, { cwd: ROOT, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] });
}

function latestTag() {
  return git('tag', '--list', 'v*', '--sort=-v:refname').split('\n')
    .map((tag) => tag.trim())
    .find((tag) => parseVersion(tag)) ?? null;
}

function fileAt(ref, file) {
  try {
    return git('show', `${ref}:${file}`);
  } catch {
    return null;
  }
}

function plan() {
  const notesText = fs.readFileSync(path.join(ROOT, NOTES_PATH), 'utf8');
  parseNotes(notesText);
  const version = parseVersion(JSON.parse(fs.readFileSync(path.join(ROOT, VERSION_PATH), 'utf8')).version);
  if (!version) {
    throw new Error(`a versao em ${VERSION_PATH} precisa ser do tipo 1.2.3`);
  }
  const tag = latestTag();
  return decide({
    version,
    latest: tag ? parseVersion(tag) : null,
    notesReleased: tag ? normalize(fileAt(tag, NOTES_PATH)) === normalize(notesText) : false,
  });
}

function main() {
  try {
    if (process.argv[2] !== 'plan') {
      throw new Error('use "plan"');
    }
    const result = plan();
    if (result.waiting) {
      process.stderr.write(`release: ha notas novas, suba a versao em ${VERSION_PATH} para publicar\n`);
    }
    process.stdout.write(`version=${result.version}\nrelease=${result.release}\n`);
  } catch (error) {
    process.stderr.write(`release: ${error.message}\n`);
    process.exitCode = 1;
  }
}

if (require.main === module) {
  main();
}

module.exports = {
  compareVersions, decide, parseNotes, parseVersion,
};
