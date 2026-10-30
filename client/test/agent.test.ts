import assert from 'node:assert/strict';
import { test } from 'node:test';
import { agentPlace } from '../src/agent.ts';

test("the agent's cursor is placed over the screen as the page scales it, from the screen's own corner", () => {
  // A 1920x1080 output drawn 960 pixels wide, 20 pixels in and 10 down in its container.
  const screen = { offsetLeft: 20, offsetTop: 10, clientWidth: 960, width: 1920 };
  assert.deepEqual(agentPlace(0, 0, screen), { left: 20, top: 10 });
  assert.deepEqual(agentPlace(1920, 1080, screen), { left: 980, top: 550 });
  assert.deepEqual(agentPlace(140, 215, screen), { left: 90, top: 117.5 });
  // Drawn larger than the output.
  assert.deepEqual(agentPlace(100, 50, { offsetLeft: 0, offsetTop: 0, clientWidth: 2560, width: 1280 }), { left: 200, top: 100 });
  // Before the screen has a size, nowhere.
  assert.equal(agentPlace(10, 10, { offsetLeft: 0, offsetTop: 0, clientWidth: 0, width: 0 }), null);
});
