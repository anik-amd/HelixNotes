import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { createContext, Script } from 'node:vm';
import ts from 'typescript';

const source = await readFile(new URL('../src/lib/ui-scale.ts', import.meta.url), 'utf8');
const script = new Script(ts.transpileModule(source.replace(/^import .*;\n/gm, ''), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 }
}).outputText);

function setup(android = true) {
  let content = 'width=device-width, initial-scale=1';
  let writes = 0;
  const listeners = [];
  const zooms = [];
  const viewport = { width: 411, scale: 1 };
  const context = createContext({
    exports: {}, isAndroid: android, console,
    screen: { width: 411, height: 850 },
    window: { visualViewport: viewport, addEventListener: (event, handler) => listeners.push({ event, handler }) },
    document: { querySelector: () => ({
      getAttribute: () => content,
      setAttribute: (_name, value) => { content = value; writes++; }
    }) },
    getCurrentWebview: () => ({ setZoom: async (scale) => zooms.push(scale) })
  });
  script.runInContext(context);
  return {
    apply: context.exports.applyUiScale, viewport, listeners, zooms,
    get writes() { return writes; },
    get values() { return Object.fromEntries(content.split(',').map((part) => part.trim().split('='))); }
  };
}

test('Android scales the layout to the visible phone width, keeping tablet layout out', async () => {
  const app = setup();
  for (const scale of [1, 1.15, 1.25, 1.4]) {
    await app.apply(scale);
    const width = Number(app.values.width);
    assert.ok(width <= 411 / scale, 'the layout must fit inside the scaled visible area');
    assert.ok(width >= 290, 'mobile menus must still fit');
    assert.ok(width < 900, 'a phone must not acquire the tablet breakpoint');
    assert.equal(Number(app.values['initial-scale']), scale);
  }
});

test('Android adapts to rotation and a narrower app window without multiplying the saved scale', async () => {
  const app = setup();
  await app.apply(1.25);
  app.viewport.width = 850 / 1.25;
  app.viewport.scale = 1.25;
  app.listeners[0].handler();
  assert.equal(Number(app.values.width), 680);
  app.viewport.width = 360 / 1.25;
  app.listeners[0].handler();
  assert.equal(Number(app.values.width), 290);
  assert.ok(Number(app.values.width) * Number(app.values['initial-scale']) <= 360);
});

test('reapplying a scale does not duplicate resize listeners or rewrite the same viewport', async () => {
  const app = setup();
  await app.apply(1.15);
  await app.apply(1.15);
  assert.equal(app.listeners.length, 1);
  assert.equal(app.listeners[0].event, 'resize');
  assert.equal(app.writes, 1);
  app.listeners[0].handler();
  assert.equal(app.writes, 1);
});

test('desktop keeps using native WebView zoom without changing the viewport', async () => {
  const app = setup(false);
  await app.apply(1.5);
  assert.deepEqual(app.zooms, [1.5]);
  assert.equal(app.writes, 0);
  assert.equal(app.listeners.length, 0);
});
