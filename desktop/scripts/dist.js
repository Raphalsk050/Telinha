'use strict';

// Empacota o app com a versao de version.json, que e onde ela e controlada. O que vier depois de
// "npm run dist --" segue para o electron-builder.

const path = require('node:path');
const { execFileSync } = require('node:child_process');

const { version } = require('../../version.json');

if (!/^\d+\.\d+\.\d+$/.test(String(version))) {
  process.stderr.write('dist: a versao em version.json precisa ser do tipo 1.2.3\n');
  process.exit(1);
}

try {
  execFileSync(process.execPath, [
    require.resolve('electron-builder/cli.js'),
    '--win', 'portable',
    `-c.extraMetadata.version=${version}`,
    ...process.argv.slice(2),
  ], { cwd: path.join(__dirname, '..'), stdio: 'inherit' });
} catch (error) {
  process.exit(error.status ?? 1);
}
