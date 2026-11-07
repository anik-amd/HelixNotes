import assert from 'node:assert/strict';
import test from 'node:test';
import { visibleMobileHeaderActionCount } from './mobile-header-actions.ts';

test('all header actions stay inline when there is room', () => {
	assert.equal(visibleMobileHeaderActionCount({ availableWidth: 400, actionCount: 5 }), 5);
	assert.equal(visibleMobileHeaderActionCount({ availableWidth: 178, actionCount: 5 }), 5);
});

test('an overflowing header reserves a slot for the three-dot button', () => {
	// 146px fits four 34px buttons with 2px gaps; one becomes the overflow button.
	assert.equal(visibleMobileHeaderActionCount({ availableWidth: 146, actionCount: 8 }), 3);
	assert.equal(visibleMobileHeaderActionCount({ availableWidth: 70, actionCount: 8 }), 1);
});

test('at least one action stays inline on a very narrow header', () => {
	assert.equal(visibleMobileHeaderActionCount({ availableWidth: -20, actionCount: 8 }), 1);
	assert.equal(visibleMobileHeaderActionCount({ availableWidth: 0, actionCount: 8 }), 1);
});

test('no actions means no inline buttons', () => {
	assert.equal(visibleMobileHeaderActionCount({ availableWidth: 400, actionCount: 0 }), 0);
});

test('custom button sizes are honored', () => {
	assert.equal(visibleMobileHeaderActionCount({
		availableWidth: 150, actionCount: 6, buttonWidth: 30, gap: 6,
	}), 3);
});

