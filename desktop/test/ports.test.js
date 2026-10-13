'use strict';

const assert = require('node:assert/strict');
const { test } = require('node:test');

const { FIRST_PORT, WINDOW_COUNT, WINDOW_SIZE, randomPortWindow } = require('../src/ports');

test('every port window stays between 51200 and 59999', () => {
  assert.deepEqual(randomPortWindow(null, () => 0), { min: 51200, max: 51219 });
  assert.deepEqual(randomPortWindow(null, () => WINDOW_COUNT - 1), { min: 59980, max: 59999 });
  for (let round = 0; round < 200; round += 1) {
    const { min, max } = randomPortWindow();
    assert.ok(min >= FIRST_PORT && max <= 59999 && max - min === WINDOW_SIZE - 1);
    assert.equal((min - FIRST_PORT) % WINDOW_SIZE, 0);
  }
});

test('a new window is never the one being left', () => {
  assert.deepEqual(randomPortWindow({ min: 51200, max: 51219 }, () => 0), { min: 51220, max: 51239 });
  assert.deepEqual(
    randomPortWindow({ min: 59980, max: 59999 }, () => WINDOW_COUNT - 1), { min: 51200, max: 51219 });
  assert.deepEqual(randomPortWindow({ min: 51200, max: 51219 }, () => 7), { min: 51340, max: 51359 });
});
