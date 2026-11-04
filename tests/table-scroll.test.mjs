import assert from 'node:assert/strict';
import test from 'node:test';
import { Table } from '@tiptap/extension-table';
import { columnResizingPluginKey, tableEditingKey, tableNodes } from '@tiptap/pm/tables';
import { EditorState } from '@tiptap/pm/state';
import { Schema } from '@tiptap/pm/model';
import { AlwaysResizableTable } from '../src/lib/editor/tableExtension.ts';

test('the table wrapper is registered even when the editor starts read-only', () => {
  const options = {
    ...Table.options,
    resizable: true,
  };
  const plugins = AlwaysResizableTable.config.addProseMirrorPlugins.call({
    options,
    editor: { isEditable: false },
  });
  const keys = new Set(plugins.map((plugin) => plugin.key));
  assert.ok(keys.has(columnResizingPluginKey.key), 'missing table node view / column resizing plugin');
  assert.ok(keys.has(tableEditingKey.key), 'missing table editing plugin');

  // The column-resizing plugin registers the table node view when its state starts. That node
  // view is what creates .tableWrapper, which owns the horizontal scrollbar.
  const resizing = plugins.find((plugin) => plugin.key === columnResizingPluginKey.key);
  const nodes = tableNodes({ tableGroup: 'block', cellContent: 'inline*', cellAttributes: {} });
  const schema = new Schema({
    nodes: {
      doc: { content: 'block+' },
      text: { group: 'inline' },
      paragraph: { content: 'inline*', group: 'block' },
      ...nodes,
    },
  });
  EditorState.create({ schema, plugins });
  assert.equal(typeof resizing.spec.props.nodeViews.table, 'function');
});

test('a non-resizable table still gets table editing', () => {
  const options = {
    ...Table.options,
    resizable: false,
  };
  const plugins = AlwaysResizableTable.config.addProseMirrorPlugins.call({
    options,
    editor: { isEditable: false },
  });
  const keys = new Set(plugins.map((plugin) => plugin.key));
  assert.equal(keys.has(columnResizingPluginKey.key), false);
  assert.ok(keys.has(tableEditingKey.key));
});
