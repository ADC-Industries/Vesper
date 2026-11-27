// A mocked Vesper for the tests: the REST routes the module calls and the WebSocket (hello,
// state, timer ticks, document changes), with synthetic content only.

const http = require('node:http')
const { WebSocketServer } = require('ws')

const IDS = {
	song: '11111111-0000-4000-8000-000000000001',
	look: '22222222-0000-4000-8000-000000000001',
	look2: '22222222-0000-4000-8000-000000000002',
	timer: '33333333-0000-4000-8000-000000000001',
	timer2: '33333333-0000-4000-8000-000000000002',
	message: '44444444-0000-4000-8000-000000000001',
	prop: '55555555-0000-4000-8000-000000000001',
	macro: '66666666-0000-4000-8000-000000000001',
	playlist: '77777777-0000-4000-8000-000000000001',
	item0: '88888888-0000-4000-8000-000000000000',
	item1: '88888888-0000-4000-8000-000000000001',
	group: '99999999-0000-4000-8000-000000000001',
}

function baseState() {
	return {
		layers: {
			videoInput: null,
			media: null,
			foreground: null,
			slide: {
				presentation: IDS.song,
				presentationName: 'Morning Light',
				slide: 'aaaaaaaa-0000-4000-8000-000000000002',
				slideIndex: 2,
				section: 'bbbbbbbb-0000-4000-8000-000000000001',
				sectionName: 'Chorus',
				playlist: { playlist: IDS.playlist, item: IDS.item1 },
				step: 2,
			},
			announcements: null,
			props: [],
			messages: [],
			audio: { asset: 'x', name: 'Prelude', kind: 'audio', online: true, playback: null },
		},
		activeLook: IDS.look,
		transport: { layer: 'audio', name: 'Prelude', state: 'playing', position: 31.5, duration: 184.2 },
		timers: [
			{ id: IDS.timer, name: 'Walk-in', state: 'running', currentSeconds: 287, display: '4:47' },
			{ id: IDS.timer2, name: 'Walk-in', state: 'stopped', currentSeconds: 60, display: '1:00' },
		],
		stageMessage: null,
		cursor: {
			presentation: IDS.song,
			presentationName: 'Morning Light',
			slideIndex: 3,
			sectionName: 'Bridge',
		},
		audio: { muted: false },
	}
}

/**
 * Starts the mock. `requireKey`: everything but `/server` and pairing needs the key `KEY`.
 * Returns `{port, commands, state, push(msg), setState(s), approve(), close()}`.
 */
async function startMock({ requireKey = false } = {}) {
	const KEY = 'idn1.test-key'
	const commands = []
	let state = baseState()
	let revision = 1
	let approved = false
	const authed = (req) => !requireKey || req.headers.authorization === `Bearer ${KEY}`
	const lists = {
		'/api/v1/looks': [
			{ id: IDS.look, name: 'Lyrics' },
			{ id: IDS.look2, name: 'Clean' },
		],
		'/api/v1/clear-groups': [{ id: IDS.group, name: 'Clear Slide' }],
		'/api/v1/timers': [
			{ id: IDS.timer, name: 'Walk-in', revision: '1', schemaVersion: 1 },
			{ id: IDS.timer2, name: 'Walk-in', revision: '1', schemaVersion: 1 },
		],
		'/api/v1/messages': [{ id: IDS.message, name: 'Parking', revision: '1', schemaVersion: 1 }],
		'/api/v1/props': [{ id: IDS.prop, name: 'Logo', revision: '1', schemaVersion: 1 }],
		'/api/v1/macros': [{ id: IDS.macro, name: 'Start service', schemaVersion: 1 }],
		'/api/v1/playlists': [
			{
				id: 'f0000000-0000-4000-8000-000000000001',
				name: 'Services',
				kind: 'folder',
				purpose: null,
				itemCount: 0,
				children: [{ id: IDS.playlist, name: 'Sunday', kind: 'playlist', purpose: 'service', itemCount: 2, children: [] }],
			},
		],
		'/api/v1/presentations': [{ id: IDS.song, name: 'Morning Light' }],
	}
	const send = (res, status, body) => {
		res.writeHead(status, { 'Content-Type': 'application/json' })
		res.end(body === undefined ? '' : JSON.stringify(body))
	}
	const server = http.createServer((req, res) => {
		let raw = ''
		req.on('data', (c) => (raw += c))
		req.on('end', () => {
			const path = req.url.split('?')[0]
			const body = raw ? JSON.parse(raw) : undefined
			if (req.method === 'GET' && path === '/api/v1/server') {
				return send(res, 200, { id: 'srv', name: 'Booth', version: '0.1.0', protocolVersion: 1, capabilities: ['ws', 'commands', 'macros'], auth: authed(req) ? (requireKey ? 'paired' : 'local') : 'unpaired' })
			}
			if (req.method === 'POST' && path === '/api/v1/pair') {
				if (body?.code === '123456') return send(res, 200, { token: KEY, clientId: 'c1' })
				if (body?.code) return send(res, 401, { error: { code: 'unauthorized', message: 'wrong code' } })
				return send(res, 202, { pairingId: 'p1', expiresInSeconds: 600 })
			}
			if (path === '/api/v1/pair/p1') {
				if (req.method === 'DELETE') return send(res, 204)
				return approved ? send(res, 200, { token: KEY, clientId: 'c1' }) : send(res, 202, { pairingId: 'p1', expiresInSeconds: 600 })
			}
			if (!authed(req)) return send(res, 401, { error: { code: 'unauthorized', message: 'pair first' } })
			if (req.method === 'POST' && path === '/api/v1/commands') {
				commands.push(body)
				revision += 1
				return send(res, 200, { ok: true, revision })
			}
			if (req.method === 'GET' && path === `/api/v1/playlists/${IDS.playlist}`) {
				return send(res, 200, {
					id: IDS.playlist,
					name: 'Sunday',
					parent: null,
					purpose: 'service',
					items: [
						{ id: IDS.item0, displayName: 'Welcome', enabled: true, kind: { type: 'header' }, presentation: null },
						{ id: IDS.item1, displayName: 'Morning Light', enabled: true, kind: { type: 'presentation' }, presentation: null },
					],
				})
			}
			if (req.method === 'GET' && lists[path]) return send(res, 200, lists[path])
			send(res, 404, { error: { code: 'notFound', message: `no route ${req.method} ${path}` } })
		})
	})
	const wss = new WebSocketServer({ noServer: true })
	server.on('upgrade', (req, socket, head) => {
		const url = new URL(req.url, 'http://x')
		if (url.pathname !== '/api/v1/ws') return socket.destroy()
		if (requireKey && url.searchParams.get('token') !== KEY) {
			socket.end('HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n')
			return
		}
		wss.handleUpgrade(req, socket, head, (ws) => {
			ws.send(JSON.stringify({ type: 'hello', server: { id: 'srv', name: 'Booth', version: '0.1.0', protocolVersion: 1 } }))
			ws.send(JSON.stringify({ type: 'state', revision, state }))
		})
	})
	await new Promise((r) => server.listen(0, '127.0.0.1', r))
	const push = (m) => {
		for (const c of wss.clients) c.send(JSON.stringify(m))
	}
	return {
		port: server.address().port,
		KEY,
		IDS,
		commands,
		push,
		setState(s) {
			state = s
			revision += 1
			push({ type: 'state', revision, state })
		},
		get state() {
			return state
		},
		approve() {
			approved = true
		},
		/** Replaces one of the lists (`/api/v1/looks`, ...). */
		setList(path, list) {
			lists[path] = list
		},
		/** A restarted Vesper: the sockets drop and the revision counts from 1 again. */
		restart(s = baseState()) {
			state = s
			revision = 1
			for (const c of wss.clients) c.terminate()
		},
		close() {
			for (const c of wss.clients) c.terminate()
			wss.close()
			server.closeAllConnections?.()
			return new Promise((r) => server.close(r))
		},
	}
}

module.exports = { startMock, baseState, IDS }
