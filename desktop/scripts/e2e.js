'use strict';

const {
  TelinhaSession,
  buildShareArgs,
  buildWatchArgs,
  listTargets,
  resolveTelinhaExe,
} = require('../src/telinha-process');

const CONNECT_TIMEOUT_MS = 90000;
const LIVE_MS = Number(process.env.TELINHA_E2E_LIVE_MS ?? 15000);

function waitForExit(session) {
  return session.exited ? Promise.resolve() : new Promise((resolve) => session.once('exit', resolve));
}

async function main() {
  const exe = resolveTelinhaExe();
  if (!exe) {
    throw new Error('telinha.exe nao encontrado, compile com cmake --build --preset msvc-release');
  }

  const targets = await listTargets(exe);
  const monitor = targets.monitors.find((candidate) => !candidate.primary) ?? targets.monitors[0];
  if (!monitor) {
    throw new Error('nenhum monitor para compartilhar');
  }
  console.log(`compartilhando ${monitor.name} ${monitor.width}x${monitor.height}`);

  const summary = {
    sharer: { state: null, stats: null, errors: [] },
    viewer: { state: null, stats: null, errors: [] },
  };
  const pending = { invite: null, viewerWantsInvite: false, answer: null, sharerWantsAnswer: false };

  const sharer = new TelinhaSession(exe, buildShareArgs({
    targetKind: 'monitor',
    targetHandle: monitor.handle,
    targetIndex: monitor.index,
    audio: 'none',
  }));
  const viewer = new TelinhaSession(exe, buildWatchArgs({ audio: false }));

  const flush = () => {
    if (pending.invite && pending.viewerWantsInvite) {
      console.log(`convite entregue ao receptor (${pending.invite.length} bytes)`);
      viewer.submitCode(pending.invite);
      pending.invite = null;
      pending.viewerWantsInvite = false;
    }
    if (pending.answer && pending.sharerWantsAnswer) {
      console.log(`resposta entregue ao emissor (${pending.answer.length} bytes)`);
      sharer.submitCode(pending.answer);
      pending.answer = null;
      pending.sharerWantsAnswer = false;
    }
  };

  const outcome = new Promise((resolve, reject) => {
    let liveTimer = null;
    const deadline = setTimeout(() => reject(new Error('as duas pontas nao conectaram a tempo')),
      CONNECT_TIMEOUT_MS);

    const track = (name, session, onCode, onNeedCode) => {
      session.on('event', (event) => {
        const entry = summary[name];
        if (event.event === 'code') {
          onCode(event.code);
        } else if (event.event === 'need_code') {
          onNeedCode();
        } else if (event.event === 'state') {
          entry.state = event.state;
          console.log(`${name}: ${event.state}`);
        } else if (event.event === 'stats') {
          entry.stats = event;
        } else if (event.event === 'error' || event.event === 'code_rejected') {
          entry.errors.push(event);
          console.log(`${name}: ${event.event} ${JSON.stringify(event)}`);
        } else if (event.event === 'step') {
          console.log(`${name}: ${event.text}`);
        }
        flush();

        if (!liveTimer && summary.sharer.state === 'Connected' && summary.viewer.state === 'Connected') {
          clearTimeout(deadline);
          liveTimer = setTimeout(resolve, LIVE_MS);
        }
      });
      session.on('exit', (info) => {
        if (!info.stopped) {
          clearTimeout(deadline);
          clearTimeout(liveTimer);
          reject(new Error(`${name} saiu antes da hora (${JSON.stringify(info)})`));
        }
      });
    };

    track('sharer', sharer, (code) => { pending.invite = code; }, () => { pending.sharerWantsAnswer = true; });
    track('viewer', viewer, (code) => { pending.answer = code; }, () => { pending.viewerWantsInvite = true; });
  });

  viewer.start();
  sharer.start();

  let failure = null;
  try {
    await outcome;
  } catch (error) {
    failure = error;
  }

  sharer.stop();
  viewer.stop();
  await Promise.all([waitForExit(sharer), waitForExit(viewer)]);

  const sent = summary.sharer.stats ? summary.sharer.stats.frames : 0;
  const presented = summary.viewer.stats ? summary.viewer.stats.presented : 0;
  console.log(`quadros enviados ${sent}, quadros exibidos ${presented}`);

  if (failure) {
    throw failure;
  }
  if (sent === 0 || presented === 0) {
    throw new Error('conectou, mas o video nao chegou a ser exibido');
  }
  console.log('ok: as duas pontas conectaram e o video foi exibido');
}

main().catch((error) => {
  console.error(`falhou: ${error.message}`);
  process.exitCode = 1;
});
