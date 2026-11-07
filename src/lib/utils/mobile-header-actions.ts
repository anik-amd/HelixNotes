export const MOBILE_HEADER_BUTTON_WIDTH = 34;
export const MOBILE_HEADER_BUTTON_GAP = 2;

export interface MobileHeaderActionFit {
	availableWidth: number;
	actionCount: number;
	buttonWidth?: number;
	gap?: number;
}

/**
 * Number of note actions that fit in the mobile header. When the row overflows,
 * one slot is reserved for the three-dot overflow button, and at least one
 * action stays visible so the header is never empty.
 */
export function visibleMobileHeaderActionCount({
	availableWidth,
	actionCount,
	buttonWidth = MOBILE_HEADER_BUTTON_WIDTH,
	gap = MOBILE_HEADER_BUTTON_GAP,
}: MobileHeaderActionFit): number {
	if (actionCount <= 0) return 0;
	const stride = buttonWidth + gap;
	const fit = Math.floor((Math.max(0, availableWidth) + gap) / stride);
	if (fit >= actionCount) return actionCount;
	return Math.max(1, fit - 1);
}

