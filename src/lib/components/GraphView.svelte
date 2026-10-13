<script lang="ts">
	import { onDestroy, untrack } from 'svelte';
	import { getGraphData } from '$lib/api';
	import { activeNotePath, appConfig } from '$lib/stores/app';
	import { isMobile } from '$lib/platform';
	import { applyGraphRepulsion } from '$lib/utils/graph-layout';
	import { buildGraphTopology, getGraphNeighborhood } from '$lib/utils/graph-topology';

	let { onclose, onnavigate }: {
		onclose: () => void;
		onnavigate: (path: string, title: string) => void;
	} = $props();

	let canvas = $state<HTMLCanvasElement>(null!);
	let loading = $state(true);
	let searchQuery = $state('');
	let localMode = $state(true); // default: show only active note's neighborhood

	const dataPromise = getGraphData($appConfig!.active_vault!);

	interface GraphNode { id: string; title: string; path: string; x: number; y: number; vx: number; vy: number; }
	interface GraphEdge { sourceIdx: number; targetIdx: number; bidirectional: boolean; }
	interface LabelRect { x: number; y: number; w: number; h: number; }

	let nodes = $state.raw<GraphNode[]>([]);
	let edges = $state.raw<GraphEdge[]>([]);
	let nodeIndexMap: Map<string, number> = new Map();
	let connectedSet: Set<number> = new Set();
	let nodeDegree: number[] = [];
	let incidentEdges = $state.raw<number[][]>([]);
	let searchMatchSet = $state.raw<Set<number> | null>(null);
	let folderColorMap = new Map<string, string>();

	let pan = { x: 0, y: 0 };
	let zoom = 1;
	let dragging: GraphNode | null = null;
	let dragMoved = false;
	let mouseDownPos = { x: 0, y: 0 };
	let panning = false;
	let panStart = { x: 0, y: 0 };
	let hoveredNode: GraphNode | null = null;
	let hoveredIdx = -1;
	let hoveredNeighborSet: Set<number> = new Set();
	let hoveredEdgeSet: Set<number> = new Set();
	let activeNodeIdx = $state(-1);
	// Svelte caches this until the active node or graph topology changes.
	let localNeighborhood = $derived(getGraphNeighborhood(activeNodeIdx, edges, incidentEdges));
	let navigatedFromGraph = false;

	// Physics runs in short yielded batches; the completed graph is static.
	let physicsRemaining = 0;
	let physicsStep: Generator<void> | null = null;
	const layoutChannel = new MessageChannel();
	let layoutScheduled = false;
	let disposed = false;
	let drawFrame: number | null = null;

	// DPI fix
	let dpr = 1;
	let canvasW = 0;
	let canvasH = 0;

	// Style cache - cleared on theme change
	let cachedStyles: { border: string; text: string; textSec: string; accent: string; bg: string } | null = null;

	const FOLDER_PALETTE = [
		'#60a5fa', '#34d399', '#f59e0b', '#a78bfa',
		'#fb7185', '#22d3ee', '#fb923c', '#86efac',
	];

	function getStyles() {
		if (cachedStyles) return cachedStyles;
		const s = getComputedStyle(document.documentElement);
		cachedStyles = {
			border:  s.getPropertyValue('--border-color').trim() || '#444',
			text:    s.getPropertyValue('--text-primary').trim() || '#eee',
			textSec: s.getPropertyValue('--text-secondary').trim() || '#aaa',
			accent:  s.getPropertyValue('--accent').trim() || '#7b9bd4',
			bg:      s.getPropertyValue('--bg-primary').trim() || '#1a1a1a',
		};
		return cachedStyles;
	}

	function getFolderKey(path: string): string {
		// Normalize Windows backslashes so folder grouping/colours work there too.
		const p = (path || '').replace(/\\/g, '/');
		const vaultRoot = ($appConfig?.active_vault || '').replace(/\\/g, '/');
		const rel = vaultRoot && p.startsWith(vaultRoot + '/') ? p.slice(vaultRoot.length + 1) : p;
		const slash = rel.indexOf('/');
		return slash >= 0 ? rel.slice(0, slash) : '';
	}

	function buildFolderColors() {
		const seen = new Map<string, string>();
		let colorIdx = 0;
		for (const n of nodes) {
			const key = getFolderKey(n.path);
			if (key && !seen.has(key)) {
				seen.set(key, FOLDER_PALETTE[colorIdx % FOLDER_PALETTE.length]);
				colorIdx++;
			}
		}
		folderColorMap = seen;
	}

	function getNodeBaseColor(i: number): string {
		const { textSec, border } = getStyles();
		if ((nodeDegree[i] || 0) === 0) return border;
		const key = getFolderKey(nodes[i].path);
		return folderColorMap.get(key) || textSec;
	}

	function getNodeRadius(i: number, isActive: boolean): number {
		if (isActive) return 8;
		const deg = nodeDegree[i] || 0;
		if (deg === 0) return 3;
		return Math.min(4.5 + deg * 0.7, 9);
	}

	function updateHoveredState(node: GraphNode | null) {
		hoveredNode = node;
		hoveredIdx = node ? nodes.indexOf(node) : -1;
		const neighborhood = getGraphNeighborhood(hoveredIdx, edges, incidentEdges);
		hoveredNeighborSet = neighborhood.nodeSet;
		hoveredEdgeSet = neighborhood.edgeSet;
	}

	function updateSearchMatch() {
		const q = searchQuery.trim().toLowerCase();
		if (!q) { searchMatchSet = null; return; }
		const matched = new Set<number>();
		nodes.forEach((n, i) => { if (n.title.toLowerCase().includes(q)) matched.add(i); });
		searchMatchSet = matched;
	}

	async function buildGraph() {
		loading = true;
		try {
			const data = await dataPromise;
			if (disposed) return;
			const w = canvasW || 800;
			const h = canvasH || 600;

			nodeIndexMap = new Map();
			nodes = data.nodes.map((n, i) => {
				nodeIndexMap.set(n.title.toLowerCase(), i);
				return {
					id: n.title.toLowerCase(),
					title: n.title,
					path: n.path,
					x: w / 2 + (Math.random() - 0.5) * Math.min(w, h) * 0.5,
					y: h / 2 + (Math.random() - 0.5) * Math.min(w, h) * 0.5,
					vx: 0, vy: 0,
				};
			});

			edges = data.edges.map(e => ({
				sourceIdx: e.source, targetIdx: e.target, bidirectional: e.bidirectional,
			}));
			const topology = buildGraphTopology(nodes.length, edges);
			connectedSet = topology.connectedSet;
			nodeDegree = topology.nodeDegree;
			incidentEdges = topology.incidentEdges;

			buildFolderColors();
			updateSearchMatch();
			activeNodeIdx = nodes.findIndex(n => n.path === ($activeNotePath || ''));
		} catch (e) {
			console.error('Failed to build graph:', e);
		}
		startSimulation();
	}

	function centerOnActiveNote() {
		if (!canvas || nodes.length === 0 || activeNodeIdx < 0) return;
		const neighborhood = localNeighborhood.nodeSet;
		const w = canvasW, h = canvasH, padding = 100;
		let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
		for (const i of neighborhood) {
			const n = nodes[i];
			if (n.x < minX) minX = n.x; if (n.y < minY) minY = n.y;
			if (n.x > maxX) maxX = n.x; if (n.y > maxY) maxY = n.y;
		}
		const graphW = maxX - minX || 1, graphH = maxY - minY || 1;
		zoom = Math.max(Math.min((w - padding * 2) / graphW, (h - padding * 2) / graphH, 2), 0.4);
		pan.x = w / 2 - ((minX + maxX) / 2) * zoom;
		pan.y = h / 2 - ((minY + maxY) / 2) * zoom;
	}

	function fitToView() {
		if (!canvas || nodes.length === 0) return;
		const w = canvasW, h = canvasH, padding = 70;
		let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
		for (const n of nodes) {
			if (n.x < minX) minX = n.x; if (n.y < minY) minY = n.y;
			if (n.x > maxX) maxX = n.x; if (n.y > maxY) maxY = n.y;
		}
		const graphW = maxX - minX || 1, graphH = maxY - minY || 1;
		zoom = Math.max(Math.min((w - padding * 2) / graphW, (h - padding * 2) / graphH, 2), 0.2);
		pan.x = w / 2 - ((minX + maxX) / 2) * zoom;
		pan.y = h / 2 - ((minY + maxY) / 2) * zoom;
	}

	function focusActive() {
		if (activeNodeIdx >= 0) centerOnActiveNote(); else fitToView();
		requestDraw();
	}

	function startSimulation() {
		// Keep the original initial layout; omit the animated settling tail.
		physicsRemaining = nodes.length > 0 ? 80 : 0;
		physicsStep = null;
		if (physicsRemaining === 0) {
			loading = false;
			requestDraw();
		} else {
			startLayoutLoop();
		}
	}

	function startLayoutLoop() {
		if (disposed || layoutScheduled || physicsRemaining === 0) return;

		function loop() {
			layoutScheduled = false;
			if (disposed) return;
			// Yield between repulsion rows, including within a large physics step.
			const deadline = performance.now() + 8;
			do {
				physicsStep ??= simulate();
				if (physicsStep.next().done) {
					physicsStep = null;
					physicsRemaining--;
				}
			} while (physicsRemaining > 0 && performance.now() < deadline);

			if (physicsRemaining === 0) {
				loading = false;
				if (activeNodeIdx >= 0) centerOnActiveNote(); else fitToView();
				requestDraw();
			} else {
				layoutScheduled = true;
				layoutChannel.port2.postMessage(null);
			}
		}
		// A queued task yields to input without waiting for a display frame or timer clamp.
		layoutChannel.port1.onmessage = loop;
		layoutScheduled = true;
		layoutChannel.port2.postMessage(null);
	}

	function* simulate(): Generator<void> {
		const graphNodes = nodes;
		const nodeCount = graphNodes.length;
		if (nodeCount === 0) return;
		const w = canvasW || 800, h = canvasH || 600;
		const centerX = w / 2, centerY = h / 2;

		yield* applyGraphRepulsion(graphNodes);

		// Spring attraction along edges
		for (const edge of edges) {
			const a = graphNodes[edge.sourceIdx], b = graphNodes[edge.targetIdx];
			const dx = b.x - a.x, dy = b.y - a.y;
			const dist = Math.sqrt(dx * dx + dy * dy) || 1;
			const force = (dist - 130) * 0.012;
			const fx = (dx / dist) * force, fy = (dy / dist) * force;
			a.vx += fx; a.vy += fy; b.vx -= fx; b.vy -= fy;
		}

		// Center gravity - weaker for isolated nodes so they spread instead of clustering
		for (let i = 0; i < nodeCount; i++) {
			const node = graphNodes[i];
			const gravity = (nodeDegree[i] || 0) === 0 ? 0.0002 : 0.0008;
			node.vx += (centerX - node.x) * gravity;
			node.vy += (centerY - node.y) * gravity;
		}

		// Damping + integration
		for (const node of graphNodes) {
			if (node === dragging) continue;
			node.vx *= 0.85; node.vy *= 0.85;
			node.x += node.vx; node.y += node.vy;
		}
	}

	function requestDraw() {
		if (disposed || drawFrame !== null) return;
		drawFrame = requestAnimationFrame(() => {
			drawFrame = null;
			draw();
		});
	}

	function draw() {
		if (disposed || !canvas || loading) return;
		const ctx = canvas.getContext('2d');
		if (!ctx) return;

		ctx.clearRect(0, 0, canvas.width, canvas.height);
		ctx.save();
		ctx.scale(dpr, dpr);
		ctx.translate(pan.x, pan.y);
		ctx.scale(zoom, zoom);

		const { border: borderColor, text: textColor, textSec, accent, bg: bgColor } = getStyles();
		const activePath = $activeNotePath || '';
		const isHovering = hoveredIdx >= 0;
		const isSearching = searchMatchSet !== null;

		// Local mode: restrict which nodes/edges are visible
		const localNodeSet = localMode ? localNeighborhood.nodeSet : null;
		const localEdgeSet = localMode ? localNeighborhood.edgeSet : null;
		// Edge strokes and arrowheads keep their screen size when zoomed out; scaled with the
		// graph they thin out below a pixel and the links fade away from the nodes.
		const edgeScale = 1 / Math.min(zoom, 1);

		// ── Edges ──────────────────────────────────────────────────────────────
		if (isHovering) {
			// Dimmed non-connected edges
			ctx.globalAlpha = 0.06;
			ctx.strokeStyle = borderColor;
			ctx.lineWidth = edgeScale;
			ctx.beginPath();
			for (let i = 0; i < edges.length; i++) {
				if (hoveredEdgeSet.has(i)) continue;
				if (localEdgeSet && !localEdgeSet.has(i)) continue;
				ctx.moveTo(nodes[edges[i].sourceIdx].x, nodes[edges[i].sourceIdx].y);
				ctx.lineTo(nodes[edges[i].targetIdx].x, nodes[edges[i].targetIdx].y);
			}
			ctx.stroke();

			// Highlighted connected edges
			ctx.globalAlpha = 0.8;
			ctx.strokeStyle = accent;
			ctx.lineWidth = 1.5 * edgeScale;
			ctx.beginPath();
			for (const i of hoveredEdgeSet) {
				// A neighbour's links to notes outside the local graph have no node to end on.
				if (localEdgeSet && !localEdgeSet.has(i)) continue;
				ctx.moveTo(nodes[edges[i].sourceIdx].x, nodes[edges[i].sourceIdx].y);
				ctx.lineTo(nodes[edges[i].targetIdx].x, nodes[edges[i].targetIdx].y);
			}
			ctx.stroke();

			// Arrowheads on highlighted edges
			ctx.fillStyle = accent;
			ctx.globalAlpha = 0.8;
			const al = 7 * edgeScale, aw = 3.5 * edgeScale;
			const drawArrow = (tipX: number, tipY: number, ux: number, uy: number) => {
				const bx = tipX - ux * al, by = tipY - uy * al;
				ctx.beginPath();
				ctx.moveTo(tipX, tipY);
				ctx.lineTo(bx - uy * aw, by + ux * aw);
				ctx.lineTo(bx + uy * aw, by - ux * aw);
				ctx.closePath();
				ctx.fill();
			};
			for (const i of hoveredEdgeSet) {
				if (localEdgeSet && !localEdgeSet.has(i)) continue;
				const e = edges[i];
				const src = nodes[e.sourceIdx];
				const tgt = nodes[e.targetIdx];
				const dx = tgt.x - src.x, dy = tgt.y - src.y;
				const dist = Math.sqrt(dx * dx + dy * dy);
				if (dist < 1) continue;
				const ux = dx / dist, uy = dy / dist;
				// Arrow at target end (source → target)
				const tgtR = getNodeRadius(e.targetIdx, tgt.path === activePath);
				drawArrow(tgt.x - ux * tgtR, tgt.y - uy * tgtR, ux, uy);
				// If bidirectional, also draw arrow at source end (target → source)
				if (e.bidirectional) {
					const srcR = getNodeRadius(e.sourceIdx, src.path === activePath);
					drawArrow(src.x + ux * srcR, src.y + uy * srcR, -ux, -uy);
				}
			}
		} else if (isSearching) {
			// All visible edges faint
			ctx.strokeStyle = borderColor;
			ctx.lineWidth = edgeScale;
			ctx.globalAlpha = 0.07;
			ctx.beginPath();
			for (let i = 0; i < edges.length; i++) {
				if (localEdgeSet && !localEdgeSet.has(i)) continue;
				ctx.moveTo(nodes[edges[i].sourceIdx].x, nodes[edges[i].sourceIdx].y);
				ctx.lineTo(nodes[edges[i].targetIdx].x, nodes[edges[i].targetIdx].y);
			}
			ctx.stroke();
			ctx.globalAlpha = 0.4;
			ctx.beginPath();
			for (let i = 0; i < edges.length; i++) {
				if (localEdgeSet && !localEdgeSet.has(i)) continue;
				const e = edges[i];
				if (searchMatchSet!.has(e.sourceIdx) || searchMatchSet!.has(e.targetIdx)) {
					ctx.moveTo(nodes[e.sourceIdx].x, nodes[e.sourceIdx].y);
					ctx.lineTo(nodes[e.targetIdx].x, nodes[e.targetIdx].y);
				}
			}
			ctx.stroke();
		} else {
			ctx.strokeStyle = borderColor;
			ctx.lineWidth = (isMobile ? 1.5 : 1) * edgeScale;
			ctx.globalAlpha = isMobile ? 0.65 : 0.35;
			ctx.beginPath();
			for (let i = 0; i < edges.length; i++) {
				if (localEdgeSet && !localEdgeSet.has(i)) continue;
				ctx.moveTo(nodes[edges[i].sourceIdx].x, nodes[edges[i].sourceIdx].y);
				ctx.lineTo(nodes[edges[i].targetIdx].x, nodes[edges[i].targetIdx].y);
			}
			ctx.stroke();

			// On mobile draw arrowheads always (no hover available)
			// Sizes divided by zoom so they stay fixed screen size regardless of zoom level
			if (isMobile) {
				ctx.fillStyle = borderColor;
				ctx.globalAlpha = 0.6;
				const al = 8 / zoom, aw = 4 / zoom;
				const drawArrow = (tipX: number, tipY: number, ux: number, uy: number) => {
					const bx = tipX - ux * al, by = tipY - uy * al;
					ctx.beginPath();
					ctx.moveTo(tipX, tipY);
					ctx.lineTo(bx - uy * aw, by + ux * aw);
					ctx.lineTo(bx + uy * aw, by - ux * aw);
					ctx.closePath();
					ctx.fill();
				};
				for (let i = 0; i < edges.length; i++) {
					if (localEdgeSet && !localEdgeSet.has(i)) continue;
					const e = edges[i];
					const src = nodes[e.sourceIdx], tgt = nodes[e.targetIdx];
					const dx = tgt.x - src.x, dy = tgt.y - src.y;
					const dist = Math.sqrt(dx * dx + dy * dy);
					if (dist < 1) continue;
					const ux = dx / dist, uy = dy / dist;
					const tgtR = getNodeRadius(e.targetIdx, tgt.path === activePath);
					drawArrow(tgt.x - ux * tgtR, tgt.y - uy * tgtR, ux, uy);
					if (e.bidirectional) {
						const srcR = getNodeRadius(e.sourceIdx, src.path === activePath);
						drawArrow(src.x + ux * srcR, src.y + uy * srcR, -ux, -uy);
					}
				}
			}
		}
		ctx.globalAlpha = 1;

		// ── Nodes + label collection ────────────────────────────────────────────
		type LabelSpec = { x: number; y: number; text: string; fontSize: number; weight: string; color: string; alpha: number; priority: number; };
		const labelQueue: LabelSpec[] = [];

		for (let i = 0; i < nodes.length; i++) {
			// Skip nodes outside local view
			if (localNodeSet && !localNodeSet.has(i)) continue;

			const node = nodes[i];
			const isActive = node.path === activePath;
			const isHovered = node === hoveredNode;
			const isNeighbor = hoveredNeighborSet.has(i);
			const hasLinks = connectedSet.has(i);
			const matchesSearch = !isSearching || searchMatchSet!.has(i);

			const dimBySearch = isSearching && !matchesSearch;
			const dimByHover = isHovering && !isActive && !isNeighbor;
			const dimmed = dimBySearch || dimByHover;

			const radius = getNodeRadius(i, isActive);

			// Keep the active note highlighted without redrawing an idle graph.
			if (isActive) {
				const glowR = radius + 12;
				const glow = ctx.createRadialGradient(node.x, node.y, radius, node.x, node.y, glowR);
				glow.addColorStop(0, accent + '80');
				glow.addColorStop(1, accent + '00');
				ctx.globalAlpha = 1;
				ctx.beginPath();
				ctx.arc(node.x, node.y, glowR, 0, Math.PI * 2);
				ctx.fillStyle = glow;
				ctx.fill();
			}

			// Node fill
			ctx.globalAlpha = dimmed ? (dimBySearch ? 0.07 : 0.12) : 1;
			ctx.beginPath();
			ctx.arc(node.x, node.y, radius, 0, Math.PI * 2);
			if (isActive || isHovered) {
				ctx.fillStyle = accent;
			} else if (isHovering && isNeighbor) {
				ctx.fillStyle = accent + 'bb';
			} else {
				ctx.fillStyle = getNodeBaseColor(i);
			}
			ctx.fill();

			// Static active outer ring.
			if (isActive) {
				ctx.globalAlpha = 0.5;
				ctx.beginPath();
				ctx.arc(node.x, node.y, radius + 3, 0, Math.PI * 2);
				ctx.strokeStyle = accent;
				ctx.lineWidth = 1.5;
				ctx.stroke();
			}

			ctx.globalAlpha = 1;

			// Queue label (skip for heavily dimmed nodes)
			if (dimBySearch) continue;
			const showLabel = isActive || isHovered ||
				(matchesSearch && isSearching) ||
				(hasLinks && zoom > 0.45);
			if (!showLabel) continue;

			const raw = node.title;
			const text = raw.length > 24 ? raw.slice(0, 22) + '…' : raw;
			const fontSize = isActive ? 12 : isHovered ? 11 : 10;
			const weight = isActive ? '600' : '400';
			const color = isActive || isHovered ? textColor : textSec;
			const alpha = dimByHover ? 0 : 1;
			const priority = isActive ? 0 : isHovered ? 1 : (matchesSearch && isSearching ? 2 : 3);

			labelQueue.push({ x: node.x, y: node.y - radius - 5, text, fontSize, weight, color, alpha, priority });
		}

		// Draw labels with collision avoidance - high-priority labels drawn first
		labelQueue.sort((a, b) => a.priority - b.priority);
		const drawnRects: LabelRect[] = [];
		ctx.textAlign = 'center';
		ctx.textBaseline = 'bottom';

		for (const lab of labelQueue) {
			if (lab.alpha === 0) continue;
			ctx.font = `${lab.weight} ${lab.fontSize}px -apple-system, BlinkMacSystemFont, sans-serif`;
			const tw = ctx.measureText(lab.text).width;
			const th = lab.fontSize + 2;
			const rx = lab.x - tw / 2, ry = lab.y - th;
			const overlaps = drawnRects.some(r =>
				rx < r.x + r.w + 6 && rx + tw > r.x - 6 &&
				ry < r.y + r.h + 3 && ry + th > r.y - 3
			);
			if (!overlaps) {
				drawnRects.push({ x: rx, y: ry, w: tw, h: th });
				ctx.globalAlpha = lab.alpha;
				ctx.fillStyle = lab.color;
				ctx.shadowColor = bgColor;
				ctx.shadowBlur = 5 / zoom;
				ctx.fillText(lab.text, lab.x, lab.y);
			}
		}
		ctx.shadowBlur = 0;
		ctx.globalAlpha = 1;
		ctx.textBaseline = 'alphabetic';

		ctx.restore();
	}

	function getNodeAt(clientX: number, clientY: number): GraphNode | null {
		if (!canvas || loading) return null;
		const rect = canvas.getBoundingClientRect();
		const x = (clientX - rect.left - pan.x) / zoom;
		const y = (clientY - rect.top - pan.y) / zoom;
		for (let i = nodes.length - 1; i >= 0; i--) {
			const n = nodes[i];
			const dx = n.x - x, dy = n.y - y;
			const r = getNodeRadius(i, n.path === ($activeNotePath || '')) + 4;
			if (dx * dx + dy * dy < r * r) return n;
		}
		return null;
	}

	function handleMouseDown(e: MouseEvent) {
		mouseDownPos = { x: e.clientX, y: e.clientY };
		dragMoved = false;
		const node = getNodeAt(e.clientX, e.clientY);
		if (node) { dragging = node; }
		else { panning = true; panStart = { x: e.clientX - pan.x, y: e.clientY - pan.y }; }
	}

	function handleMouseMove(e: MouseEvent) {
		const dx = e.clientX - mouseDownPos.x, dy = e.clientY - mouseDownPos.y;
		if (Math.abs(dx) > 3 || Math.abs(dy) > 3) dragMoved = true;

		if (dragging && dragMoved) {
			const rect = canvas.getBoundingClientRect();
			dragging.x = (e.clientX - rect.left - pan.x) / zoom;
			dragging.y = (e.clientY - rect.top - pan.y) / zoom;
			dragging.vx = 0; dragging.vy = 0;
			requestDraw();
		} else if (panning) {
			pan.x = e.clientX - panStart.x;
			pan.y = e.clientY - panStart.y;
			requestDraw();
		} else if (!dragging) {
			const node = getNodeAt(e.clientX, e.clientY);
			if (node !== hoveredNode) {
				updateHoveredState(node);
				if (canvas) canvas.style.cursor = node ? 'pointer' : 'grab';
				if (!layoutScheduled) requestDraw();
			}
		}
	}

	function handleMouseUp(e: MouseEvent) {
		if (dragging && !dragMoved) {
			const node = dragging;
			dragging = null;
			navigatedFromGraph = true;
			onnavigate(node.path, node.title);
			return;
		}
		dragging = null;
		panning = false;
	}

	function handleWheel(e: WheelEvent) {
		e.preventDefault();
		const rect = canvas.getBoundingClientRect();
		const mx = e.clientX - rect.left, my = e.clientY - rect.top;
		const oldZoom = zoom;
		zoom = Math.max(0.2, Math.min(5, zoom * (e.deltaY > 0 ? 0.9 : 1.1)));
		pan.x = mx - (mx - pan.x) * (zoom / oldZoom);
		pan.y = my - (my - pan.y) * (zoom / oldZoom);
		requestDraw();
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			if (searchQuery) { searchQuery = ''; return; }
			onclose();
		}
	}

	let lastTouchDist = 0;

	function handleTouchStart(e: TouchEvent) {
		if (e.touches.length === 1) {
			const t = e.touches[0];
			mouseDownPos = { x: t.clientX, y: t.clientY };
			dragMoved = false;
			const node = getNodeAt(t.clientX, t.clientY);
			if (node) { dragging = node; }
			else { panning = true; panStart = { x: t.clientX - pan.x, y: t.clientY - pan.y }; }
		} else if (e.touches.length === 2) {
			dragging = null; panning = false;
			const dx = e.touches[0].clientX - e.touches[1].clientX;
			const dy = e.touches[0].clientY - e.touches[1].clientY;
			lastTouchDist = Math.sqrt(dx * dx + dy * dy);
		}
	}

	function handleTouchMove(e: TouchEvent) {
		e.preventDefault();
		if (e.touches.length === 1) {
			const t = e.touches[0];
			const dx = t.clientX - mouseDownPos.x, dy = t.clientY - mouseDownPos.y;
			if (Math.abs(dx) > 3 || Math.abs(dy) > 3) dragMoved = true;
			if (dragging && dragMoved) {
				const rect = canvas.getBoundingClientRect();
				dragging.x = (t.clientX - rect.left - pan.x) / zoom;
				dragging.y = (t.clientY - rect.top - pan.y) / zoom;
				dragging.vx = 0; dragging.vy = 0;
				requestDraw();
			} else if (panning) {
				pan.x = t.clientX - panStart.x;
				pan.y = t.clientY - panStart.y;
				requestDraw();
			}
		} else if (e.touches.length === 2) {
			const dx = e.touches[0].clientX - e.touches[1].clientX;
			const dy = e.touches[0].clientY - e.touches[1].clientY;
			const dist = Math.sqrt(dx * dx + dy * dy);
			if (lastTouchDist > 0) {
				const midX = (e.touches[0].clientX + e.touches[1].clientX) / 2;
				const midY = (e.touches[0].clientY + e.touches[1].clientY) / 2;
				const rect = canvas.getBoundingClientRect();
				const mx = midX - rect.left, my = midY - rect.top;
				const oldZoom = zoom;
				zoom = Math.max(0.2, Math.min(5, zoom * (dist / lastTouchDist)));
				pan.x = mx - (mx - pan.x) * (zoom / oldZoom);
				pan.y = my - (my - pan.y) * (zoom / oldZoom);
				requestDraw();
			}
			lastTouchDist = dist;
		}
	}

	function handleTouchEnd(e: TouchEvent) {
		if (e.touches.length === 0) {
			if (dragging && !dragMoved) {
				const node = dragging;
				dragging = null;
				navigatedFromGraph = true;
				onnavigate(node.path, node.title);
				return;
			}
			dragging = null; panning = false; lastTouchDist = 0;
		}
	}

	// Canvas setup
	$effect(() => {
		if (canvas) {
			const rect = canvas.parentElement?.getBoundingClientRect();
			if (rect) {
				dpr = window.devicePixelRatio || 1;
				canvasW = rect.width;
				canvasH = rect.height;
				canvas.width = canvasW * dpr;
				canvas.height = canvasH * dpr;
			}
			buildGraph();
		}
	});

	// React to active note changes (e.g., user clicks node → navigates → re-center)
	$effect(() => {
		const ap = $activeNotePath;
		const newIdx = nodes.findIndex(n => n.path === (ap || ''));
		activeNodeIdx = newIdx;
		if (navigatedFromGraph && newIdx >= 0 && canvas) {
			navigatedFromGraph = false;
			centerOnActiveNote();
		}
		if (!layoutScheduled && canvas) untrack(requestDraw);
	});

	// React to search query changes
	$effect(() => {
		const _q = searchQuery; // track
		updateSearchMatch();
		if (canvas && !layoutScheduled) untrack(requestDraw);
	});

	onDestroy(() => {
		disposed = true;
		if (drawFrame !== null) cancelAnimationFrame(drawFrame);
		drawFrame = null;
		layoutChannel.port1.onmessage = null;
		layoutChannel.port1.close();
		layoutChannel.port2.close();
	});
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="graph-overlay" onkeydown={handleKeydown}>
	<div class="graph-panel" class:mobile={isMobile}>
		<div class="graph-header">
			<h3>Graph View</h3>
			<div class="graph-search">
				<svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>
				</svg>
				<!-- svelte-ignore a11y_autofocus -->
				<input
					type="text"
					bind:value={searchQuery}
					placeholder="Filter notes..."
					class:active={!!searchQuery}
				/>
				{#if searchQuery}
					<button class="graph-search-clear" aria-label="Clear search" onclick={() => { searchQuery = ''; }}>
						<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round">
							<line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
						</svg>
					</button>
				{/if}
			</div>
			<div class="graph-stats">
				{#if !loading}
					{#if localMode && activeNodeIdx >= 0}
						{localNeighborhood.nodeSet.size} notes · {localNeighborhood.edgeSet.size} links
					{:else}
						{nodes.length} notes · {edges.length} links
					{/if}
					{#if searchMatchSet}
						· <span class="search-count">{searchMatchSet.size} matched</span>
					{/if}
				{/if}
			</div>
			<button
				class="graph-btn"
				class:active={localMode}
				title={localMode ? 'Show all notes' : 'Show local graph'}
				onclick={() => { localMode = !localMode; focusActive(); }}
			>
				<!-- network/local icon -->
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<circle cx="12" cy="5" r="2"/><circle cx="5" cy="19" r="2"/><circle cx="19" cy="19" r="2"/>
					<line x1="12" y1="7" x2="5" y2="17"/><line x1="12" y1="7" x2="19" y2="17"/>
				</svg>
			</button>
			<button class="graph-btn" onclick={focusActive} title="Focus active note">
				<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<circle cx="12" cy="12" r="3"/>
					<line x1="12" y1="2" x2="12" y2="6"/>
					<line x1="12" y1="18" x2="12" y2="22"/>
					<line x1="2" y1="12" x2="6" y2="12"/>
					<line x1="18" y1="12" x2="22" y2="12"/>
				</svg>
			</button>
			<button class="graph-close" aria-label="Close graph" onclick={onclose}>
				<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
					<line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/>
				</svg>
			</button>
		</div>
		<div class="graph-body">
			{#if loading}
				<div class="graph-loading">
					<svg class="spinner" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
						<circle cx="12" cy="12" r="10" opacity="0.25"/>
						<path d="M12 2a10 10 0 019.95 9"/>
					</svg>
					Building graph...
				</div>
			{/if}
			<!-- svelte-ignore a11y_no_static_element_interactions -->
			<canvas
				bind:this={canvas}
				class="graph-canvas"
				onmousedown={handleMouseDown}
				onmousemove={handleMouseMove}
				onmouseup={handleMouseUp}
				onwheel={handleWheel}
				ontouchstart={handleTouchStart}
				ontouchmove={handleTouchMove}
				ontouchend={handleTouchEnd}
			></canvas>
		</div>
	</div>
</div>

<style>
	.graph-overlay {
		position: fixed;
		inset: 0;
		background: rgba(0, 0, 0, 0.5);
		display: flex;
		align-items: center;
		justify-content: center;
		z-index: 2000;
	}

	.graph-panel {
		background: var(--bg-primary);
		border: 1px solid var(--border-color);
		border-radius: 16px;
		box-shadow: var(--shadow-lg);
		width: 85vw;
		height: 75vh;
		max-width: 1200px;
		max-height: 800px;
		display: flex;
		flex-direction: column;
		overflow: hidden;
	}

	.graph-header {
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 12px 16px;
		border-bottom: 1px solid var(--border-light);
		flex-shrink: 0;
	}

	.graph-panel.mobile {
		width: 100vw;
		height: 100vh;
		max-width: none;
		max-height: none;
		border-radius: 0;
		border: none;
	}

	.graph-panel.mobile .graph-header {
		padding-top: calc(env(safe-area-inset-top, 36px) + 12px);
	}

	.graph-panel.mobile .graph-header h3 {
		display: none;
	}

	.graph-panel.mobile .graph-stats {
		display: none;
	}

	.graph-panel.mobile .graph-search {
		max-width: none;
	}

	.graph-header h3 {
		font-size: 14px;
		font-weight: 600;
		color: var(--text-primary);
		margin: 0;
		white-space: nowrap;
	}

	.graph-search {
		display: flex;
		align-items: center;
		gap: 6px;
		flex: 1;
		max-width: 260px;
		background: var(--bg-secondary);
		border: 1px solid var(--border-light);
		border-radius: 8px;
		padding: 4px 8px;
		color: var(--text-tertiary);
		transition: border-color 0.15s;
	}

	.graph-search:focus-within {
		border-color: var(--accent);
		color: var(--text-secondary);
	}

	.graph-search input {
		background: none;
		border: none;
		outline: none;
		font-size: 12px;
		color: var(--text-primary);
		flex: 1;
		min-width: 0;
	}

	.graph-search input::placeholder {
		color: var(--text-tertiary);
	}

	.graph-search-clear {
		background: none;
		border: none;
		padding: 0;
		cursor: pointer;
		color: var(--text-tertiary);
		display: flex;
		align-items: center;
		flex-shrink: 0;
	}

	.graph-search-clear:hover {
		color: var(--text-primary);
	}

	.graph-stats {
		flex: 1;
		font-size: 11px;
		color: var(--text-tertiary);
		white-space: nowrap;
	}

	.search-count {
		color: var(--accent);
		font-weight: 500;
	}

	.graph-btn,
	.graph-close {
		background: none;
		border: none;
		color: var(--text-tertiary);
		cursor: pointer;
		padding: 4px;
		border-radius: 6px;
		display: flex;
		align-items: center;
		flex-shrink: 0;
	}

	.graph-btn.active {
		color: var(--accent);
		background: var(--accent-light);
	}

	.graph-btn:hover,
	.graph-close:hover {
		background: var(--bg-hover);
		color: var(--text-primary);
	}

	.graph-body {
		flex: 1;
		position: relative;
		overflow: hidden;
	}

	.graph-canvas {
		width: 100%;
		height: 100%;
		cursor: grab;
		touch-action: none;
	}

	.graph-canvas:active {
		cursor: grabbing;
	}

	.graph-loading {
		position: absolute;
		inset: 0;
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 10px;
		font-size: 13px;
		color: var(--text-tertiary);
		z-index: 1;
	}

	.spinner {
		animation: spin 1s linear infinite;
	}

	@keyframes spin {
		to { transform: rotate(360deg); }
	}
</style>
