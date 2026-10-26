import assert from 'node:assert/strict';
import { test } from 'node:test';
import { screenFor } from '../src/ui/screens.ts';

test("a session's screen is a common desktop size of the device's shape, near the window's width", () => {
  // A 14-inch MacBook (16:10-ish), in a window and full screen.
  assert.deepEqual(screenFor(1512, 982, 1512), { width: 1440, height: 900 });
  assert.deepEqual(screenFor(1512, 982, 1280), { width: 1280, height: 800 });
  // A 1080p monitor, a wide window and a narrow one.
  assert.deepEqual(screenFor(1920, 1080, 1900), { width: 1920, height: 1080 });
  assert.deepEqual(screenFor(1920, 1080, 1350), { width: 1366, height: 768 });
  assert.deepEqual(screenFor(1920, 1080, 1300), { width: 1280, height: 720 });
  // A 5:4 and a 4:3 one.
  assert.deepEqual(screenFor(1280, 1024, 1200), { width: 1280, height: 1024 });
  assert.deepEqual(screenFor(1024, 768, 1000), { width: 1024, height: 768 });
});
