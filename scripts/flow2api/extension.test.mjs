import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';

const sent = [];
let calls = 0;
let created = 0;
let closed = 0;
class WebSocketMock {
  static OPEN = 1;
  static CONNECTING = 0;
  readyState = 1;
  constructor() { this.current = this; }
  send(message) { sent.push(JSON.parse(message)); }
}

const context = {
  WebSocket: WebSocketMock,
  URL,
  console: { log() {}, error() {} },
  setTimeout: (fn, ms) => setTimeout(fn, ms === 1500 ? 0 : ms),
  clearTimeout,
  setInterval: () => 0,
  clearInterval() {},
  chrome: {
    storage: {
      local: { get(_defaults, callback) { callback({ serverUrl: 'ws://127.0.0.1:8765/captcha_ws', apiKey: 'test' }); }, set() {} },
      onChanged: { addListener() {} }
    },
    runtime: { onStartup: { addListener() {} }, lastError: null },
    tabs: {
      query: async () => [{ id: 1, status: 'complete', url: 'https://flow.google.com/' }],
      create: async () => { created++; return { id: 2 }; },
      get(_id, callback) { callback({ status: 'complete' }); },
      onUpdated: { addListener() {}, removeListener() {} },
      remove: async () => { closed++; }
    },
    scripting: { executeScript: async () => ++calls === 1 ? [{ result: undefined }] : [{ result: 'test-token' }] }
  }
};

vm.runInNewContext(readFileSync(new URL('../../flow2api-src/extension/background.js', import.meta.url), 'utf8'), context);
await new Promise((resolve) => setImmediate(resolve));
await vm.runInNewContext('handleGetToken({ req_id: "one", action: "IMAGE_GENERATION" })', context);
assert.equal(calls, 2);
assert.equal(created, 1);
assert.equal(closed, 1);
assert.equal(sent.at(-1).status, 'success');
assert.equal(sent.at(-1).token, 'test-token');
