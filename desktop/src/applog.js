'use strict';

const fs = require('node:fs');
const path = require('node:path');

const MAX_BYTES = 512 * 1024;

// Registro do app em arquivo. E o que sobra para entender por que o Telinha nao abriu, fechou
// sozinho ou nao se atualizou no computador de outra pessoa.
class AppLog {
  constructor(file) {
    this.file = file;
  }

  // Passando do limite, fica so a metade mais nova.
  trim() {
    let size = 0;
    try {
      size = fs.statSync(this.file).size;
    } catch {
      return;
    }
    if (size <= MAX_BYTES) {
      return;
    }
    const text = fs.readFileSync(this.file, 'utf8');
    const half = text.slice(text.length / 2);
    fs.writeFileSync(this.file, half.slice(half.indexOf('\n') + 1));
  }

  write(level, text) {
    const body = String(text).replace(/\r?\n/g, '\n    ');
    try {
      fs.mkdirSync(path.dirname(this.file), { recursive: true });
      this.trim();
      fs.appendFileSync(this.file, `${new Date().toISOString()} ${level} ${body}\n`);
    } catch {
      // sem disco ou sem permissao, o app segue sem registro
    }
  }

  info(text) {
    this.write('info', text);
  }

  error(text) {
    this.write('erro', text);
  }
}

function describeError(error) {
  return error && error.stack ? error.stack : String(error);
}

module.exports = { AppLog, describeError };
