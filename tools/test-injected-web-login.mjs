// Run with: node tools/test-injected-web-login.mjs
// Exercises the actual injected script without accessing an account or network.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
const source = readFileSync(new URL('../src/relay/injected/web_login.js', import.meta.url), 'utf8');
function page({ origin = 'https://injected-irc.com', child = false } = {}) {
  const storage = new Map();
  const messages = [];
  const state = { poll: undefined, ok: true, username: 'alice', calls: 0, changedDuringFetch: false };
  const window = { addEventListener() {}, ipc: { postMessage: body => messages.push(JSON.parse(body)) } };
  window.top = child ? {} : window;
  const context = {
    window, location: { origin },
    localStorage: { getItem: key => storage.get(key) ?? null },
    setInterval: fn => { state.poll = fn; },
    fetch: async (url, options) => {
      state.calls++;
      assert.equal(url, '/api/me');
      assert.equal(options.redirect, 'error');
      if (state.changedDuringFetch) storage.set('injected_token', 'replacement');
      return { ok: state.ok, json: async () => ({ username: state.username }) };
    }
  };
  vm.runInNewContext(source, context);
  return { storage, messages, state };
}
for (const options of [{ origin: 'https://example.org' }, { child: true }]) {
  assert.equal(page(options).state.poll, undefined, 'foreign origins and frames must have no bridge');
}
const login = page();
await login.state.poll();
assert.equal(login.state.calls, 0, 'no verification request before sign-in completes');
login.storage.set('injected_token', 'fake-session');
login.state.ok = false;
await login.state.poll();
assert.equal(login.messages.length, 0, 'a rejected token must not be returned');
login.state.ok = true;
await login.state.poll();
await login.state.poll();
assert.deepEqual(login.messages, [{ token: 'fake-session' }], 'return a verified session once');
const race = page();
race.storage.set('injected_token', 'old');
race.state.changedDuringFetch = true;
await race.state.poll();
assert.equal(race.messages.length, 0, 'ignore a token replaced during verification');
race.state.changedDuringFetch = false;
await race.state.poll();
assert.deepEqual(race.messages, [{ token: 'replacement' }]);
const invalid = page();
invalid.storage.set('injected_token', 'fake');
invalid.state.username = undefined;
await invalid.state.poll();
assert.equal(invalid.messages.length, 0, 'require an authenticated account response');
console.log('Injected sign-in bridge: all functional checks passed.');
