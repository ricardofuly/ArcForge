const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

function setup() {
  const elements = new Map();
  const frames = [];
  const calls = [];
  const toasts = [];
  let listener;
  let finish;
  const invocation = new Promise((resolve, reject) => { finish = { resolve, reject }; });
  const context = vm.createContext({
    navigator: { userAgent: 'Windows' },
    document: { getElementById(id) {
      if (!elements.has(id)) elements.set(id, {
        textContent: '', disabled: false, hidden: true, value: 0,
        addEventListener(type, handler) { this[type] = handler; },
        removeAttribute(name) { delete this[name]; },
      });
      return elements.get(id);
    } },
    window: { __TAURI__: {
      core: { invoke: (command, args) => { calls.push({ command, args }); return invocation; } },
      event: { listen: async (event, callback) => { listener = callback; return () => {}; } },
      dialog: { open() {} }, opener: { openUrl() {} },
    } },
    requestAnimationFrame: (callback) => { frames.push(callback); return frames.length; },
    showToast: (...args) => toasts.push(args),
  });
  const source = fs.readFileSync(path.join(__dirname, '../src/main.js'), 'utf8');
  vm.runInContext(source.slice(0, source.indexOf('\nlet engines =')), context);
  vm.runInContext('buildUiReady = initBuildUi()', context);
  return {
    calls, toasts, finish, elements,
    launch: (name = 'Game') => vm.runInContext(`launchProject({name:${JSON.stringify(name)},uproject_path:'C:/Projeto/Game.uproject'},'engine','auto')`, context),
    emit: (payload) => listener({ payload: { uproject_path: 'C:/Projeto/Game.uproject', ...payload } }),
    flush: () => { while (frames.length) frames.shift()(); },
  };
}

test('waits for compilation, handles progress, blocks duplicate opening and keeps the log', async () => {
  const ui = setup();
  const launch = ui.launch();
  await new Promise(setImmediate);
  const modal = ui.elements.get('project-build-modal');
  const close = ui.elements.get('btn-close-project-build');
  const bar = ui.elements.get('project-build-progress');
  assert.equal(modal.hidden, false);
  assert.equal(close.disabled, true);
  assert.equal(bar.value, undefined);
  assert.equal(ui.calls.length, 1);
  ui.emit({ stage: 'building', percent: 25, line: '<script>unsafe</script>', log_path: 'C:/Logs/build.log' });
  ui.flush();
  assert.equal(bar.value, 25);
  assert.equal(ui.elements.get('project-build-log').textContent, '<script>unsafe</script>');
  ui.emit({ uproject_path: 'C:/Other.uproject', percent: 80, line: 'unrelated' });
  assert.equal(bar.value, 25);
  await ui.launch();
  assert.equal(ui.calls.length, 1);
  close.click();
  assert.equal(modal.hidden, false);
  ui.finish.resolve();
  await launch;
  assert.equal(bar.value, 100);
  assert.equal(close.disabled, false);
  assert.equal(ui.elements.get('btn-show-build-log').disabled, false);
  close.click();
  assert.equal(modal.hidden, true);
});

test('errors remain visible and the screen bounds log memory while disk retains the full output', async () => {
  const ui = setup();
  const launch = ui.launch();
  await new Promise(setImmediate);
  for (let i = 0; i < 2100; i++) ui.emit({ stage: 'building', line: `line ${i}`, log_path: 'C:/Logs/build.log' });
  ui.emit({ stage: 'failed', line: 'error C2039: missing symbol', log_path: 'C:/Logs/build.log' });
  ui.finish.reject(new Error('Build failed (6)'));
  await launch;
  ui.flush();
  const log = ui.elements.get('project-build-log').textContent;
  assert.ok(log.includes('error C2039'));
  assert.ok(log.includes('Build failed (6)'));
  assert.equal(log.split('\n').length, 2000);
  assert.equal(ui.elements.get('project-build-modal').hidden, false);
  assert.equal(ui.elements.get('btn-close-project-build').disabled, false);
  assert.equal(ui.elements.get('project-build-progress').value, 0);
});

test('Blueprint projects can finish without a build log', async () => {
  const ui = setup();
  const launch = ui.launch('Blueprint');
  ui.finish.resolve();
  await launch;
  assert.equal(ui.elements.get('project-build-modal').hidden, true);
  assert.equal(ui.elements.get('btn-show-build-log').disabled, true);
});
