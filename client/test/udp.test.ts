import assert from 'node:assert/strict';
import { test } from 'node:test';
import { encode, MAX_SERVER_MESSAGE, type ServerMessage, VERSION } from '../src/protocol.ts';
import { frame, Frames, Handover, MAX_BUFFERED } from '../src/udp.ts';

const config: ServerMessage = { type: 'config', version: VERSION, epoch: 2, width: 64, height: 64, codec: 1, maxFps: 60 };
const tiles = (revision: bigint): ServerMessage => ({
  type: 'tiles',
  epoch: 2,
  revision,
  rects: [{ x: 0, y: 0, w: 1, h: 1 }],
  rawLen: 1,
  data: new Uint8Array(1),
});
const pong: ServerMessage = { type: 'time-pong', clientTimeUs: 1n, serverTimeUs: 2n };

/** A frame as the admin writes one: its length, the link's relay tag, the message. */
function framed(message: Uint8Array): Uint8Array {
  const out = new Uint8Array(5 + message.length);
  new DataView(out.buffer).setUint32(0, message.length + 1, true);
  out.set(message, 5);
  return out;
}

test('the media stream splits into messages however its chunks fall', () => {
  const a = Uint8Array.from([0x0a, 1, 2, 3]);
  const b = Uint8Array.from([0x0b, 9]);
  const stream = Uint8Array.from([...framed(a), ...framed(b)]);
  for (let cut = 0; cut <= stream.length; cut++) {
    const frames = new Frames();
    const got = [...frames.push(stream.slice(0, cut)), ...frames.push(stream.slice(cut))];
    assert.deepEqual(got, [a, b], `cut at ${cut}`);
  }
});

test('a frame no server sends breaks the media stream', () => {
  const header = (len: number, tag: number): Uint8Array => {
    const out = new Uint8Array(5);
    new DataView(out.buffer).setUint32(0, len, true);
    out[4] = tag;
    return out;
  };
  assert.throws(() => new Frames().push(header(0, 0)));
  assert.throws(() => new Frames().push(header(MAX_SERVER_MESSAGE + 2, 0)));
  assert.throws(() => new Frames().push(header(2, 1)), /not a relayed message/);
});

test('what the client sends over the session is framed as the server reads it', () => {
  const ack = { type: 'media-ack', revision: 7n } as const;
  assert.deepEqual(frame(ack), framed(encode(ack)));
});

test("the session's media wait, unacknowledged, until the WebSocket says they moved", () => {
  const handover = new Handover();
  handover.opened(3);
  assert.deepEqual(handover.fromPath(tiles(5n), 100), []);
  assert.deepEqual(handover.fromPath(tiles(6n), 100), []);
  // Another offer's word moves nothing.
  assert.deepEqual(handover.movedTo(2), []);
  assert.deepEqual(handover.movedTo(3), [tiles(5n), tiles(6n)]);
  assert.equal(handover.moved, true);
  assert.deepEqual(handover.fromPath(tiles(7n), 100), [tiles(7n)]);
  assert.deepEqual(handover.movedTo(3), [], 'once');
});

test('too much waiting for the word gives the session up', () => {
  const handover = new Handover();
  handover.opened(1);
  assert.deepEqual(handover.fromPath(tiles(1n), MAX_BUFFERED), []);
  assert.equal(handover.fromPath(tiles(2n), 1), null);
});

test('after a fallback the media wait for the new configuration; nothing else does', () => {
  const handover = new Handover();
  handover.opened(1);
  handover.movedTo(1);
  handover.fellBack();
  assert.equal(handover.generation, null);
  assert.equal(handover.moved, false);
  assert.equal(handover.takes(tiles(9n)), false, 'media of an epoch the client may never have seen');
  assert.equal(handover.takes(pong), true);
  assert.equal(handover.takes(config), true);
  assert.equal(handover.takes(tiles(10n)), true);
});

test('a session that closes before the media moved drops what waited', () => {
  const handover = new Handover();
  handover.opened(4);
  handover.fromPath(tiles(1n), 10);
  handover.closed();
  assert.deepEqual(handover.movedTo(4), []);
  assert.equal(handover.takes(tiles(2n)), true);
});

test('a message much larger than the buffer, in small chunks, arrives whole', () => {
  const big = Uint8Array.from({ length: 300_000 }, (_, i) => i % 251);
  big[0] = 0x0a;
  const stream = Uint8Array.from([...framed(Uint8Array.of(0x0b)), ...framed(big), ...framed(Uint8Array.of(0x0c))]);
  const frames = new Frames();
  const got: Uint8Array[] = [];
  for (let at = 0; at < stream.length; at += 1000) got.push(...frames.push(stream.slice(at, at + 1000)));
  assert.deepEqual(got, [Uint8Array.of(0x0b), big, Uint8Array.of(0x0c)]);
});
