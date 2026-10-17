import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { transformWithEsbuild } from 'vite';

const source = await readFile(new URL('../src/lib/editor/renamedLinks.ts', import.meta.url), 'utf8');
const { code } = await transformWithEsbuild(source, 'renamedLinks.ts', { loader: 'ts', format: 'esm' });
const { renamedLinkChanges } = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);

const apply = (text, rename) => {
  let result = text;
  for (const change of renamedLinkChanges(text, rename).reverse()) {
    result = result.slice(0, change.from) + change.insert + result.slice(change.to);
  }
  return result;
};

test('links to a renamed note follow it, as the backend rewrites them', () => {
  // Same input and result as the backend test for update_wikilinks_after_rename.
  const rename = { old_ref: 'Home', new_ref: 'Start', old_title: 'Home', new_title: 'Start', title_rules: true };
  assert.equal(
    apply('See [[Home]], [[Home|the start page]] and [[Home|Home]].\nAlso [[Index|Home]] and [[Notes/Home|Home]].\n', rename),
    'See [[Start]], [[Start|the start page]] and [[Start|Start]].\nAlso [[Index|Home]] and [[Notes/Home|Home]].\n'
  );
});

test('path links follow a rename in a folder', () => {
  const rename = { old_ref: 'Work/Plan', new_ref: 'Work/Roadmap', old_title: 'Plan', new_title: 'Roadmap', title_rules: true };
  assert.equal(
    apply('[[Work/Plan]] [[Work/Plan|Plan]] [[Work/Plan|mine]] [[Plan]] [[Other/Plan]]', rename),
    '[[Work/Roadmap]] [[Work/Roadmap|Roadmap]] [[Work/Roadmap|mine]] [[Roadmap]] [[Other/Plan]]'
  );
});

test('title links stay when another note shares the old title', () => {
  const rename = { old_ref: 'Work/Plan', new_ref: 'Work/Roadmap', old_title: 'Plan', new_title: 'Roadmap', title_rules: false };
  assert.equal(apply('[[Plan]] [[Plan|x]] [[Work/Plan|Plan]]', rename), '[[Plan]] [[Plan|x]] [[Work/Roadmap|Roadmap]]');
});

test('nothing changes when no link points at the renamed note', () => {
  const rename = { old_ref: 'Home', new_ref: 'Start', old_title: 'Home', new_title: 'Start', title_rules: true };
  assert.deepEqual(renamedLinkChanges('Home is [[Elsewhere]] and `code`.', rename), []);
});

test('aliased links in a table cell follow a rename', () => {
  const rename = { old_ref: 'Home', new_ref: 'Start', old_title: 'Home', new_title: 'Start', title_rules: true };
  assert.equal(
    apply('| [[Home\\|Home]] | [[Home\\|start]] and [[Index\\|Home]] |', rename),
    '| [[Start\\|Start]] | [[Start\\|start]] and [[Index\\|Home]] |'
  );
});

// The same cases are checked against the backend rewrite in operations.rs.
const fixtures = JSON.parse(await readFile(new URL('./fixtures/renamed-links.json', import.meta.url), 'utf8'));
for (const fixture of fixtures) {
  test(`shared rename case: ${fixture.name}`, () => {
    let result = fixture.input;
    for (const change of renamedLinkChanges(fixture.input, fixture.renames ?? fixture.rename, fixture.note_dir).reverse()) {
      result = result.slice(0, change.from) + change.insert + result.slice(change.to);
    }
    assert.equal(result, fixture.expected);
  });
}
