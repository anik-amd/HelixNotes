// Drives the Android app through Chrome DevTools Protocol to verify the
// mobile editor header: vault open, note creation, Note Info panel, overflow menu.
const BASE = 'http://127.0.0.1:9222';
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const log = (...args) => console.log(...args);
const consoleErrors = [];
let failures = 0;

function fail(message) {
	failures += 1;
	console.error('CHECK FAILED:', message);
}

async function pageTarget() {
	for (let i = 0; i < 60; i += 1) {
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

async function main() {
	const target = await pageTarget();
	log(`target: ${target.title} ${target.url}`);
	const cdp = await Cdp.open(target.webSocketDebuggerUrl);

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

	const bodyText = () => cdp.evaluate('document.body.innerText.slice(0, 6000)');

	async function waitFor(label, expression, timeout = 30000) {
		const deadline = Date.now() + timeout;
		while (Date.now() < deadline) {
			if (await cdp.evaluate(expression)) return true;
			await sleep(400);
		}
		fail(`${label} did not appear`);
		log('screen text:', (await bodyText()).slice(0, 1200));
		return false;
	}

	// 1. Vault picker → open the seeded vault.
	const pickerReady = await waitFor(
		'vault picker',
		`document.body.innerText.includes('Get Started') || document.body.innerText.includes('Open Notes Folder')`,
		45000,
	);
	if (pickerReady) {
		const clicked = await cdp.evaluate(`(() => {
			const button = [...document.querySelectorAll('button')].find((b) => /Get Started|Open Notes Folder/.test(b.textContent || ''));
			if (!button) return false;
			button.click();
			return true;
		})()`);
		log('vault open click:', clicked);
		if (!clicked) fail('no vault open button');
	}

	// 2. Main mobile layout and the seeded notes.
	const layoutReady = await waitFor('mobile layout', `!!document.querySelector('.mobile-header')`, 45000);
	if (layoutReady) log('main screen text:', (await bodyText()).slice(0, 800));

	const notesVisible = await waitFor(
		'seeded notes',
		`document.body.innerText.includes('Sample Note One')`,
		30000,
	);
	if (!notesVisible) fail('existing notes were not listed');

	// 3. Create a note.
	const created = await cdp.evaluate(`(() => {
		const button = document.querySelector('[title^="New note"]');
		if (!button) return false;
		button.click();
		return true;
	})()`);
	if (!created) fail('no new note button on the note list');
	log('new note click:', created);

	const editorReady = await waitFor('editor', `!!document.querySelector('.ProseMirror')`, 30000);
	if (editorReady) {
		const title = await cdp.evaluate(`document.querySelector('.editor-title input')?.value ?? ''`);
		log('new note title:', JSON.stringify(title));
		if (!title) fail('new note opened without a title');
	}

	// 4. Note Info: inline button, otherwise through the overflow menu.
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
		await waitFor('overflow menu', `!!document.querySelector('.mobile-header-menu')`, 10000);
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
		`!!document.querySelector('.info-panel') && document.querySelector('.info-panel').innerText.includes('Note Info')`,
		15000,
	);
	if (infoOpen) {
		const stats = await cdp.evaluate(`document.querySelector('.info-panel').innerText.replace(/\\s+/g, ' ').slice(0, 300)`);
		log('info panel:', stats);
		if (!/Words/i.test(stats)) fail('info panel has no word count');
	}

	// 5. Close the panel.
	await cdp.evaluate(`document.querySelector('.info-close-btn')?.click()`);
	const infoClosed = await waitFor('info panel closed', `!document.querySelector('.info-panel')`, 10000);
	if (!infoClosed) fail('info panel did not close');

	// 6. Overflow menu contents.
	const header = await cdp.evaluate(`(() => ({
		inline: document.querySelectorAll('.mobile-header-actions .mobile-header-btn').length,
		hasMore: !!document.querySelector('[aria-label="More actions"]'),
	}))()`);
	log('header:', JSON.stringify(header));
	if (!header.hasMore) fail('header did not collapse into a More actions button on a narrow screen');

	if (header.hasMore) {
		await cdp.evaluate(`document.querySelector('[aria-label="More actions"]').click()`);
		await waitFor('overflow menu', `!!document.querySelector('.mobile-header-menu')`, 10000);
		const menu = await cdp.evaluate(`(() => {
			const items = [...document.querySelectorAll('.mobile-header-menu-item')];
			return { count: items.length, labels: items.map((item) => (item.textContent || '').trim()) };
		})()`);
		log('overflow menu:', JSON.stringify(menu));
		if (!menu.count) fail('overflow menu is empty');
		await cdp.evaluate(`document.querySelector('.mobile-header-menu-backdrop')?.click()`);
		await waitFor('menu closed', `!document.querySelector('.mobile-header-menu')`, 10000);
	}

	if (consoleErrors.length) {
		log('console errors:');
		for (const error of consoleErrors.slice(0, 20)) log(' -', String(error).slice(0, 400));
	}

	cdp.close();
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

