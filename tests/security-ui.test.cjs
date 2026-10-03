const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const root = path.join(__dirname, '..');
const source = fs.readFileSync(path.join(root, 'src/main.js'), 'utf8');

test('untrusted engine metadata is rendered as text instead of injected markup', () => {
  const cards = [];
  const list = { appendChild: card => cards.push(card) };
  const context = vm.createContext({
    navigator: { userAgent: 'Windows' },
    document: {
      getElementById: () => list,
      createElement: () => ({ querySelector: () => ({ addEventListener() {} }) }),
    },
    engines: [], currentSearchFilter: '', currentCategoryFilter: 'all',
    currentAvailableBlobs: [{ name: 'bad.zip', version: '5.8<img src=x onerror="bad()">', size: 0 }],
  });
  vm.runInContext(source.slice(source.indexOf('function escapeHtml('), source.indexOf('\n/**', source.indexOf('function escapeHtml('))), context);
  vm.runInContext(source.slice(source.indexOf('function renderAvailableEngines('), source.indexOf('\ndocument.getElementById("engine-search-filter")')), context);
  vm.runInContext('renderAvailableEngines()', context);
  assert.equal(cards.length, 1);
  assert.ok(cards[0].innerHTML.includes('&lt;img'));
  assert.ok(!cards[0].innerHTML.includes('<img'));
});

test('unsigned/unconfigured update opens fixed official release page without invoking installer', async () => {
  const opened = [];
  const context = vm.createContext({
    availableUpdate: { automatic_update_ready: false, asset_url: 'https://evil.invalid/setup.exe' },
    openUrl: async url => opened.push(url),
    invoke: () => { throw new Error('Unverified installer invoked'); },
  });
  vm.runInContext(source.slice(source.indexOf('async function startLiveUpdate('), source.indexOf('\n// Escuta o progresso do live update')), context);
  await vm.runInContext('startLiveUpdate()', context);
  assert.deepEqual(opened, ['https://github.com/ricardofuly/ArcForge/releases/latest']);
});

test('every application command is covered by ACL and granted only to the local main window', () => {
  const main = fs.readFileSync(path.join(root, 'src-tauri/src/main.rs'), 'utf8');
  const build = fs.readFileSync(path.join(root, 'src-tauri/build.rs'), 'utf8');
  const permission = fs.readFileSync(path.join(root, 'src-tauri/permissions/arcforge.toml'), 'utf8');
  const commands = [...main.matchAll(/commands::(\w+)/g)].map(m => m[1]).sort();
  const manifest = [...build.matchAll(/"(\w+)"/g)].map(m => m[1]).filter(n => commands.includes(n)).sort();
  const allowed = JSON.parse(permission.match(/commands.allow = (\[.*\])/)[1]).sort();
  assert.deepEqual(manifest, commands);
  assert.deepEqual(allowed, commands);
  const capability = JSON.parse(fs.readFileSync(path.join(root, 'src-tauri/capabilities/default.json'), 'utf8'));
  assert.deepEqual(capability.windows, ['main']);
  assert.ok(capability.permissions.includes('arcforge-main'));
  assert.equal(capability.remote, undefined);
});
