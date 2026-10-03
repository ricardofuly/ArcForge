const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const source = fs.readFileSync(path.join(__dirname, '../src/main.js'), 'utf8');

function harness(fetchCatalog = () => []) {
  const elements = new Map();
  let loginClicks = 0;
  let renderCount = 0;
  const context = vm.createContext({
    document: { getElementById(id) {
      if (!elements.has(id)) {
        const children = new Map();
        elements.set(id, { hidden: false, disabled: false, innerHTML: '', textContent: '',
          click: () => { loginClicks++; },
          querySelector(selector) {
            if (!children.has(selector)) children.set(selector, { textContent: '' });
            return children.get(selector);
          },
        });
      }
      return elements.get(id);
    } },
    currentAvailableBlobs: [], uiIcon: () => '', showToast() {},
    renderAvailableEngines: () => { renderCount++; },
    invoke: command => {
      assert.equal(command, 'epic_list_available_engines');
      return fetchCatalog();
    },
  });
  vm.runInContext(source.slice(source.indexOf('let engineDownloadAccountId'), source.indexOf('// ---------- Conta Epic Games')), context);
  vm.runInContext(source.slice(source.indexOf('async function loadEngineCatalog('), source.indexOf('// Ouve atualizações em background')), context);
  vm.runInContext(source.slice(source.indexOf('async function openEpicDownloader('), source.indexOf('// ---------- Gerenciador de Downloads')), context);
  context.refreshEpicStatus = async () => vm.runInContext('({logged_in: !!engineDownloadAccountId, account_id: engineDownloadAccountId})', context);
  return { elements, run: code => vm.runInContext(code, context), loginClicks: () => loginClicks, renderCount: () => renderCount };
}

test('anonymous engine action requests login and never opens/downloads the catalog', async () => {
  const h = harness(() => { throw new Error('Anonymous catalog access'); });
  h.run('setEngineDownloadAccount(null, true)');
  h.elements.get('download-engine-modal').hidden = true;
  assert.equal(h.elements.get('btn-open-epic-downloader').querySelector('span').textContent, 'Entrar para baixar Unreal Engine');
  await h.run('openEpicDownloader()');
  await h.run('loadEngineCatalog()');
  assert.equal(h.loginClicks(), 2);
  assert.equal(h.elements.get('download-engine-modal').hidden, true);
});

test('logout closes the catalog, clears its items and discards a pending response', async () => {
  let resolve;
  const h = harness(() => new Promise(done => { resolve = done; }));
  h.run('setEngineDownloadAccount("account-a")');
  const pending = h.run('loadEngineCatalog(true)');
  await new Promise(done => setImmediate(done));
  h.run('setEngineDownloadAccount(null, true)');
  resolve([{ name: 'Engine.zip', version: '5.8' }]);
  await pending;
  assert.equal(h.run('currentAvailableBlobs.length'), 0);
  assert.equal(h.renderCount(), 0);
  assert.equal(h.elements.get('download-engine-modal').hidden, true);
});

test('account switch prevents an earlier response from replacing the new catalog', async () => {
  const requests = [];
  const h = harness(() => new Promise(resolve => requests.push(resolve)));
  h.run('setEngineDownloadAccount("account-a")');
  const first = h.run('loadEngineCatalog(true)');
  await new Promise(done => setImmediate(done));
  h.run('setEngineDownloadAccount("account-b")');
  const second = h.run('loadEngineCatalog(true)');
  await new Promise(done => setImmediate(done));
  requests[0]([{ name: 'Old.zip' }]);
  await first;
  assert.equal(h.elements.get('btn-refresh-catalog').disabled, true);
  requests[1]([{ name: 'New.zip' }]);
  await second;
  assert.equal(h.run('currentAvailableBlobs[0].name'), 'New.zip');
  assert.equal(h.renderCount(), 1);
});
