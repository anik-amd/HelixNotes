import { getCurrentWebview } from '@tauri-apps/api/webview';
import { isAndroid } from '$lib/platform';

// Interface scale (#288). The CSS is sized in desktop px, which reads too small on phones, so
// Android starts one step up when the user hasn't picked a scale yet.
export const defaultUiScale = isAndroid ? 1.15 : 1;

// Scaling up narrows the layout viewport. Keep it at least as wide as the widest mobile menus
// (~290px) so they still fit on narrow phones. Uses the short side so rotating doesn't change it.
const minAndroidLayoutWidth = 290;
const maxAndroidUiScale =
	isAndroid && typeof screen !== 'undefined'
		? Math.max(1, Math.min(screen.width, screen.height) / minAndroidLayoutWidth)
		: Infinity;

export const uiScalePresets: { label: string; value: number }[] = isAndroid
	? [
			{ label: '100%', value: 1 },
			{ label: '115%', value: 1.15 },
			{ label: '125%', value: 1.25 },
			{ label: '140%', value: 1.4 },
		].filter((preset) => preset.value <= maxAndroidUiScale)
	: [
			{ label: '80%', value: 0.8 },
			{ label: '90%', value: 0.9 },
			{ label: '100%', value: 1 },
			{ label: '125%', value: 1.25 },
			{ label: '150%', value: 1.5 },
			{ label: '175%', value: 1.75 },
			{ label: '200%', value: 2 },
		];

// Android's WebView ignores setZoom (wry no-op), and CSS `zoom` would double-scale every popup
// positioned from clientX/getBoundingClientRect. Pinning the viewport scale instead shrinks the
// layout viewport, like the system "Display size" setting, so all coordinates stay in CSS px.
// Needs useWideViewPort, which MainActivity enables.
function applyAndroidViewportScale(scale: number) {
	const meta = document.querySelector('meta[name="viewport"]');
	if (!meta) return;
	scale = Math.min(scale, maxAndroidUiScale);
	meta.setAttribute(
		'content',
		`initial-scale=${scale}, minimum-scale=${scale}, maximum-scale=${scale}, user-scalable=no, viewport-fit=cover`
	);
}

export async function applyUiScale(scale: number) {
	if (isAndroid) {
		applyAndroidViewportScale(scale);
		return;
	}
	try {
		await getCurrentWebview().setZoom(scale);
	} catch (e) {
		console.error('Failed to apply interface scale:', e);
	}
}
