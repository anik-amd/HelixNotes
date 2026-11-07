// Drives the Android app through Chrome DevTools Protocol.
//
// Phase A proves the storage-permission gate: with "All files access" denied,
// the vault picker must offer a Grant File Access action that opens the system
// permission screen.
// Phase B proves the note flow: grant access, open the seeded vault, list notes,
// create a note, open Note Info, and use the three-dot overflow menu.
import { execSync } from 'node:child_process';

const BASE = 'http://127.0.0.1:9222';
const PKG = 'com.helixnotes.app';
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const log = (...args) => console.log(...args);
let failures = 0;

function fail(message) {
	failures += 1;
	console.error('CHECK FAILED:', message);
}

function adb(command) {
	return execSync(`adb ${command}`, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
}

function adbSoft(command) {
	try {
		return adb(command);
	} catch (error) {
		return String(error.stdout || error.message || '');
	}
}

async function waitForSocket() {
	for (let i = 0; i < 60; i += 1) {
		const out = adbSoft('shell cat /proc/net/unix');
		const match = out.match(/webview_devtools_remote_\d+/);
		if (match) return match[0];
		await sleep(500);
	}
	throw new Error('no WebView devtools socket');
}

async function pageTarget() {
	for (let i = 0; i < 90; i += 1) {
		try {
			const response = await fetch(`${BASE}/json/list`);
			const list = await response.json();
			const page = list.find((target) => target.type === 'page');
			if (page) return page;
		} catch {
			// devtools not ready yet
		}
		await sleep(500);
	}
	throw new Error('no page target on the devtools socket');
}

class Cdp {
	constructor(socket) {
		this.socket = socket;
		this.nextId = 1;
		this.pending = new Map();
		this.listeners = new Map();
		socket.onmessage = (event) => {
			const message = JSON.parse(event.data);
			if (message.id && this.pending.has(message.id)) {
				const { resolve, reject } = this.pending.get(message.id);
				this.pending.delete(message.id);
				if (message.error) reject(new Error(JSON.stringify(message.error)));
				else resolve(message.result);
				return;
			}
			const handlers = this.listeners.get(message.method) ?? [];
			for (const handler of handlers) handler(message.params);
		};
	}

	static async open(url) {
		const socket = new WebSocket(url);
		await new Promise((resolve, reject) => {
			socket.onopen = resolve;
			socket.onerror = () => reject(new Error('devtools websocket failed'));
		});
		return new Cdp(socket);
	}

	on(method, handler) {
		const handlers = this.listeners.get(method) ?? [];
		handlers.push(handler);
		this.listeners.set(method, handlers);
	}

	send(method, params = {}) {
		const id = this.nextId++;
		return new Promise((resolve, reject) => {
			this.pending.set(id, { resolve, reject });
			this.socket.send(JSON.stringify({ id, method, params }));
		});
	}

	async evaluate(expression) {
		const result = await this.send('Runtime.evaluate', {
			expression,
			awaitPromise: true,
			returnByValue: true,
			userGesture: true,
		});
		if (result.exceptionDetails) {
			throw new Error(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text);
		}
		return result.result.value;
	}

	close() {
		this.socket.close();
	}
}

async function connect() {
	adbSoft('forward --remove tcp:9222');
	const socket = await waitForSocket();
	log(`devtools socket: ${socket}`);
	adb(`forward tcp:9222 localabstract:${socket}`);
	const target = await pageTarget();
	log(`target: ${target.title} ${target.url}`);
	return Cdp.open(target.webSocketDebuggerUrl);
}

async function launchApp() {
	adb(`shell am force-stop ${PKG}`);
	await sleep(1000);
	adb(`shell am start -W -n ${PKG}/.MainActivity`);
	await sleep(6000);
}

function waitFor(label, cdp, expression, timeout = 30000) {
	return (async () => {
		const deadline = Date.now() + timeout;
		while (Date.now() < deadline) {
			try {
				if (await cdp.evaluate(expression)) return true;
			} catch {
				// page may be reloading
			}
			await sleep(400);
		}
		fail(`${label} did not appear`);
		try {
			log('screen text:', (await cdp.evaluate('document.body.innerText.slice(0, 1200)')).replace(/\n+/g, ' | '));
		} catch {
			// ignore
		}
		return false;
	})();
}

const buttonByText = (text) => `(() => {
	const button = [...document.querySelectorAll('button')].find((b) => (b.textContent || '').trim() === ${JSON.stringify(text)});
	if (!button) return false;
	button.click();
	return true;
})()`;

const primaryButtonText = `document.querySelector('.btn-primary')?.textContent?.trim() ?? ''`;

// --------------------------------------------------------------------- phase A
async function phaseA() {
	log('\n=== PHASE A: storage permission denied ===');
	adbSoft(`shell pm clear ${PKG}`);
	adbSoft(`shell appops set ${PKG} MANAGE_EXTERNAL_STORAGE deny`);
	await launchApp();

	const cdp = await connect();
	try {
		const picker = await waitFor('vault picker', cdp, `!!document.querySelector('.btn-primary')`, 45000);
		if (!picker) return;

		const primary = await cdp.evaluate(primaryButtonText);
		log('primary button:', JSON.stringify(primary));
		if (primary !== 'Grant File Access') {
			fail(`primary button should be "Grant File Access" without storage access, saw ${JSON.stringify(primary)}`);
		}

		const clicked = await cdp.evaluate(buttonByText('Grant File Access'));
		log('grant click:', clicked);
		if (!clicked) fail('no Grant File Access button');

		await sleep(4000);
		const resumed = adbSoft('shell dumpsys activity activities');
		const openedSettings = /mResumedActivity.*(Settings|permission)/i.test(resumed) || /All files access/i.test(resumed);
		log('system settings opened:', openedSettings);
		if (!openedSettings) fail('Grant File Access did not open the system permission screen');

		// Leave the settings screen and grant the permission the way a user would.
		adbSoft('shell input keyevent KEYCODE_BACK');
		await sleep(1000);
		adbSoft(`shell appops set ${PKG} MANAGE_EXTERNAL_STORAGE allow`);
		adbSoft(`shell am start -W -n ${PKG}/.MainActivity`);
		await sleep(3000);

		const ungated = await waitFor(
			'permission-aware vault picker',
			cdp,
			`document.querySelector('.btn-primary')?.textContent?.trim() === 'Get Started'`,
			30000,
		);
		if (!ungated) fail('vault picker still gated after granting storage access');
	} finally {
		cdp.close();
	}
}

// --------------------------------------------------------------------- phase B
async function phaseB() {
	log('\n=== PHASE B: vault, notes, Note Info, overflow menu ===');
	adbSoft(`shell appops set ${PKG} MANAGE_EXTERNAL_STORAGE allow`);
	await launchApp();

	const cdp = await connect();
	const consoleErrors = [];
	try {
		cdp.on('Runtime.exceptionThrown', (params) => {
			consoleErrors.push(params.exceptionDetails?.exception?.description ?? params.exceptionDetails?.text);
		});
		cdp.on('Runtime.consoleAPICalled', (params) => {
			if (params.type !== 'error') return;
			consoleErrors.push(params.args.map((arg) => arg.value ?? arg.description ?? '').join(' '));
		});
		cdp.on('Log.entryAdded', (params) => {
			if (params.entry.level === 'error') consoleErrors.push(params.entry.text);
		});
		await cdp.send('Runtime.enable');
		await cdp.send('Log.enable');

		// 1. Open the seeded vault.
		const picker = await waitFor(
			'vault picker',
			cdp,
			`document.body.innerText.includes('Get Started') || document.body.innerText.includes('Open Notes Folder')`,
			45000,
		);
		if (picker) {
			const clicked = await cdp.evaluate(buttonByText('Get Started')) ||
				(await cdp.evaluate(buttonByText('Open Notes Folder')));
			log('vault open click:', clicked);
			if (!clicked) fail('no vault open button');
		}

		// 2. Mobile layout.
		const layoutReady = await waitFor('mobile layout', cdp, `!!document.querySelector('.mobile-header')`, 45000);
		if (layoutReady) log('sidebar text:', (await cdp.evaluate('document.body.innerText.slice(0, 500)')).replace(/\n+/g, ' | '));

		// 3. Sidebar -> All Notes.
		const wentToNotes = await cdp.evaluate(`(() => {
			const item = [...document.querySelectorAll('.nav-item')].find((b) => (b.textContent || '').includes('All Notes'));
			if (!item) return false;
			item.click();
			return true;
		})()`);
		log('all notes click:', wentToNotes);
		if (!wentToNotes) fail('no All Notes nav item');

		const notesListed = await waitFor(
			'seeded notes',
			cdp,
			`document.querySelector('.mobile-panel.active')?.innerText.includes('Sample Note One') ?? false`,
			30000,
		);
		if (notesListed) {
			const panelText = await cdp.evaluate(`document.querySelector('.mobile-panel.active').innerText.replace(/\\s+/g, ' ').slice(0, 400)`);
			log('note list:', panelText);
		} else {
			fail('existing notes were not listed');
		}

		// 4. Create a note from the note list.
		const created = await cdp.evaluate(`(() => {
			const button = document.querySelector('.mobile-panel.active .mobile-create-btn');
			if (!button) return false;
			button.click();
			return true;
		})()`);
		log('new note click:', created);
		if (!created) fail('no new note button on the note list');

		const editorReady = await waitFor('editor', cdp, `!!document.querySelector('.ProseMirror')`, 30000);
		if (editorReady) {
			const title = await cdp.evaluate(`document.querySelector('.editor-title input')?.value ?? ''`);
			log('new note title:', JSON.stringify(title));
			if (!title) fail('new note opened without a title');
		}

		// 5. Note Info: inline button, otherwise through the overflow menu.
		const inlineInfo = await cdp.evaluate(`!!document.querySelector('.mobile-header-actions [aria-label="Note info"]')`);
		log('note info inline:', inlineInfo);

		if (inlineInfo) {
			await cdp.evaluate(`document.querySelector('.mobile-header-actions [aria-label="Note info"]').click()`);
		} else {
			const opened = await cdp.evaluate(`(() => {
				const more = document.querySelector('[aria-label="More actions"]');
				if (!more) return false;
				more.click();
				return true;
			})()`);
			if (!opened) fail('neither inline Note info nor a More actions button');
			await waitFor('overflow menu', cdp, `!!document.querySelector('.mobile-header-menu')`, 10000);
			const fromMenu = await cdp.evaluate(`(() => {
				const item = [...document.querySelectorAll('.mobile-header-menu-item')].find((b) => (b.textContent || '').includes('Note info'));
				if (!item) return false;
				item.click();
				return true;
			})()`);
			if (!fromMenu) fail('Note info missing from the overflow menu');
		}

		const infoOpen = await waitFor(
			'note info panel',
			cdp,
			`!!document.querySelector('.info-panel') && document.querySelector('.info-panel').innerText.includes('Note Info')`,
			15000,
		);
		if (infoOpen) {
			const stats = await cdp.evaluate(`document.querySelector('.info-panel').innerText.replace(/\\s+/g, ' ').slice(0, 300)`);
			log('info panel:', stats);
			if (!/Words/i.test(stats)) fail('info panel has no word count');
		}

		// 6. Close the panel.
		await cdp.evaluate(`document.querySelector('.info-close-btn')?.click()`);
		const infoClosed = await waitFor('info panel closed', cdp, `!document.querySelector('.info-panel')`, 10000);
		if (!infoClosed) fail('info panel did not close');

		// 7. Overflow menu contents.
		const header = await cdp.evaluate(`(() => ({
			inline: document.querySelectorAll('.mobile-header-actions .mobile-header-btn').length,
			hasMore: !!document.querySelector('[aria-label="More actions"]'),
		}))()`);
		log('header:', JSON.stringify(header));
		if (!header.hasMore) fail('header did not collapse into a More actions button on a narrow screen');

		if (header.hasMore) {
			await cdp.evaluate(`document.querySelector('[aria-label="More actions"]').click()`);
			await waitFor('overflow menu', cdp, `!!document.querySelector('.mobile-header-menu')`, 10000);
			const menu = await cdp.evaluate(`(() => {
				const items = [...document.querySelectorAll('.mobile-header-menu-item')];
				return { count: items.length, labels: items.map((item) => (item.textContent || '').trim()) };
			})()`);
			log('overflow menu:', JSON.stringify(menu));
			if (!menu.count) fail('overflow menu is empty');
			await cdp.evaluate(`document.querySelector('.mobile-header-menu-backdrop')?.click()`);
			await waitFor('menu closed', cdp, `!document.querySelector('.mobile-header-menu')`, 10000);
		}

		if (consoleErrors.length) {
			log('console errors:');
			for (const error of consoleErrors.slice(0, 20)) log(' -', String(error).slice(0, 400));
		}
	} finally {
		cdp.close();
	}
}

async function main() {
	await phaseA();
	await phaseB();
	if (failures > 0) {
		console.error(`SMOKE FAILED with ${failures} failed check(s)`);
		process.exit(1);
	}
	log('SMOKE PASSED');
}

main().catch((error) => {
	console.error('SMOKE ERROR:', error.message);
	process.exit(1);
});

