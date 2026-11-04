import { Table } from '@tiptap/extension-table';
import { columnResizing, tableEditing } from '@tiptap/pm/tables';

/**
 * TipTap registers the table node view together with its column-resizing plugin, and skips both
 * when the editor is created read-only. Without the node view there is no `.tableWrapper`, so a
 * wide table loses its horizontal scroll until the app is restarted in Edit mode.
 *
 * Keep the table view and its wrapper even when the initial state is View mode. The resizing
 * handlers already ignore non-editable views; this only makes the wrapper available to scroll.
 */
export const AlwaysResizableTable = Table.extend({
	addProseMirrorPlugins() {
		const plugins = [];
		if (this.options.resizable) {
			plugins.push(columnResizing({
				handleWidth: this.options.handleWidth,
				cellMinWidth: this.options.cellMinWidth,
				defaultCellMinWidth: this.options.cellMinWidth,
				View: this.options.View,
				lastColumnResizable: this.options.lastColumnResizable,
			}));
		}
		plugins.push(tableEditing({ allowTableNodeSelection: this.options.allowTableNodeSelection }));
		return plugins;
	},
});
