'use strict';

const crypto = require('node:crypto');

const WINDOW_SIZE = 20;
const FIRST_PORT = 51200;
const WINDOW_COUNT = 440;

// As portas UDP locais das chamadas e das transmissoes saem de uma janela sorteada, e nao mais das
// faixas fixas 50000-50039. Com a faixa fixa, todo PC e toda tentativa usavam a mesma porta. Num
// roteador que trata 50000-50100 de outro jeito, o pacote da outra pessoa que chegava antes do
// primeiro pacote daqui deixava um registro preso, a saida passava a sair por outra porta e a
// conexao direta nunca fechava, tentativa apos tentativa. Dois PCs na mesma casa tambem disputavam
// a mesma porta no roteador. Uma janela nova muda o par de portas e escapa do registro preso.
function randomPortWindow(except = null, random = crypto.randomInt) {
  let index = random(WINDOW_COUNT);
  if (except && except.min === FIRST_PORT + WINDOW_SIZE * index) {
    index = (index + 1) % WINDOW_COUNT;
  }
  const min = FIRST_PORT + WINDOW_SIZE * index;
  return { min, max: min + WINDOW_SIZE - 1 };
}

module.exports = { FIRST_PORT, WINDOW_COUNT, WINDOW_SIZE, randomPortWindow };
