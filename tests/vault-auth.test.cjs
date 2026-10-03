const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const source = fs.readFileSync(path.join(__dirname, '../src/main.js'), 'utf8');

function harness(listItems) {
  const elements = new Map();
  const context = vm.createContext({
    document: { getElementById(id) {
      if (!elements.has(id)) elements.set(id, { hidden: true, disabled: false, innerHTML: '', textContent: '', appendChild() {} });
      return elements.get(id);
    } },
    uiIcon: () => '', showToast() {}, createVaultCard: () => ({}),
    invoke: async (command) => {
      assert.equal(command, 'list_vault_items');
      return listItems();
    },
  });
  vm.runInContext(source.slice(source.indexOf('let vaultItems = []'), source.indexOf('function createVaultCard(')), context);
  context.refreshEpicStatus = async () => vm.runInContext('vaultAccountId ? { logged_in: true, account_id: vaultAccountId } : { logged_in: false }', context);
  return { context, elements, run: code => vm.runInContext(code, context) };
}

const item = { title: 'Private plugin', developer: '', description: '', releases: [], item_type: 'plugin' };

test('anonymous library never invokes cache command and shows login prompt', async () => {
  const h = harness(() => { throw new Error('Anonymous cache access'); });
  h.run('vaultItems = [{title:"stale"}]; vaultFilteredItems = vaultItems;');
  await h.run('refreshVault()');
  assert.equal(h.elements.get('vault-grid').innerHTML, '');
  assert.equal(h.elements.get('vault-empty-title').textContent, 'Conecte sua conta Epic.');
  assert.equal(h.elements.get('btn-refresh-vault').disabled, true);
});

test('logout discards pending library response and clears item/action state', async () => {
  let resolve;
  const h = harness(() => new Promise(done => { resolve = done; }));
  h.run('setVaultAccount("account-a"); selectedVaultItem = {id:"private"};');
  const pending = h.run('refreshVault()');
  await new Promise(done => setImmediate(done));
  h.run('setVaultAccount(null, true)');
  resolve([item]);
  await pending;
  assert.equal(h.run('vaultItems.length'), 0);
  assert.equal(h.run('selectedVaultItem'), null);
  assert.equal(h.run('vaultLoaded'), false);
  assert.equal(h.elements.get('vault-action-modal').hidden, true);
});

test('account switch discards old response without unlocking the newer refresh', async () => {
  const pending = [];
  const h = harness(() => new Promise(resolve => pending.push(resolve)));
  h.run('setVaultAccount("account-a")');
  const first = h.run('refreshVault()');
  await new Promise(done => setImmediate(done));
  h.run('setVaultAccount("account-b")');
  const second = h.run('refreshVault()');
  await new Promise(done => setImmediate(done));
  pending[0]([item]);
  await first;
  assert.equal(h.run('vaultItems.length'), 0);
  assert.equal(h.run('isVaultRefreshing'), true);
  pending[1]([]);
  await second;
  assert.equal(h.run('isVaultRefreshing'), false);
  assert.equal(h.run('vaultLoaded'), true);
});
