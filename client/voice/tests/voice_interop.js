'use strict';

// Two Rust apps without a window in the same voice room of a server made for this run, on the real
// message servers. Each has a tone for a microphone and plays nothing. Checks that the call
// negotiation crosses the message servers, that both connect and that each hears the other.
//
//   node client/voice/tests/voice_interop.js
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const readline = require('node:readline');
const { spawn } = require('node:child_process');

const exe = path.join(__dirname, '..', '..', 'target', 'debug', 'examples', 'voice_probe.exe');
const scratch = fs.mkdtempSync(path.join(os.tmpdir(), 'telinha-voice-'));
const seconds = process.argv[2] ?? '45';
const seen = { ana: { joined: false, connected: false, heard: false }, bia: { joined: false, connected: false, heard: false } };
const reports = [];
const running = [];

function run(name, invite) {
  const args = [path.join(scratch, name), name, seconds];
  if (invite) {
    args.push(invite);
  }
  const child = spawn(exe, args, { stdio: ['ignore', 'pipe', 'inherit'] });
  running.push(new Promise((resolve) => child.on('exit', resolve)));
  readline.createInterface({ input: child.stdout }).on('line', (line) => {
    let event;
    try {
      event = JSON.parse(line);
    } catch {
      return;
    }
    if (event.event === 'ready') {
      run('bia', event.invite);
    } else if (event.event === 'joined') {
      seen[name].joined = true;
    } else if (event.event === 'peer' && event.connected) {
      seen[name].connected = true;
    } else if (event.event === 'heard' && event.speaking) {
      seen[name].heard = true;
    } else if (event.event === 'report') {
      reports.push(`${name}: ${event.line}`);
    }
  });
}

run('ana', null);
setTimeout(async () => {
  await Promise.all(running);
  fs.rmSync(scratch, { recursive: true, force: true });
  console.log(JSON.stringify(seen));
  console.log(reports.join('\n'));
  const ok = Object.values(seen).every((side) => side.joined && side.connected && side.heard);
  console.log(ok ? 'ok: a chamada conecta e as duas pontas se ouvem' : 'falhou');
  process.exit(ok ? 0 : 1);
}, (Number(seconds) + 4) * 1000);
