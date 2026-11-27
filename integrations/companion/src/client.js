// Vesper's API as the module uses it: REST (`/api/v1/...`) for pairing, lists and commands, and
// one WebSocket (`/api/v1/ws`) for the show state. See `crates/protocol/README.md` in the Vesper
// repository. No Companion types here, so the tests can drive it against a mocked server.

const { EventEmitter } = require('node:events')
const WebSocket = require('ws')

const PROTOCOL_VERSION = 1

/** An API error: the HTTP status and the API's `{error: {code, message}}`. */
class VesperError extends Error {
	constructor(status, code, message) {
		super(message || `HTTP ${status}`)
		this.status = status
		this.code = code || 'unknown'
	}
}

/** Document kinds whose lists the module shows in dropdowns. */
const LIST_KINDS = new Set(['look', 'clearGroup', 'timer', 'message', 'prop', 'macro', 'playlistNode', 'presentation'])

class VesperClient extends EventEmitter {
	/**
	 * @param {{host: string, port: number, key?: string, reconnectMs?: number}} opts
	 */
	constructor(opts) {
		super()
		this.host = String(opts.host || '').trim()
		this.port = Number(opts.port) || 5555
		this.key = (opts.key || '').trim()
		this.reconnectMs = opts.reconnectMs ?? 2000
		this.ws = null
		this.closed = false
		this.timer = null
		this.state = null
		this.revision = 0
	}

	get baseUrl() {
		const h = this.host.includes(':') && !this.host.startsWith('[') ? `[${this.host}]` : this.host
		return `http://${h}:${this.port}`
	}

	headers(json) {
		const h = {}
		if (json) h['Content-Type'] = 'application/json'
		if (this.key) h.Authorization = `Bearer ${this.key}`
		return h
	}

	/** A REST request: the JSON answer (`null` for an empty one), else a `VesperError`. */
	async request(method, path, body) {
		let res
		try {
			res = await fetch(this.baseUrl + '/api/v1' + path, {
				method,
				headers: this.headers(body !== undefined),
				body: body === undefined ? undefined : JSON.stringify(body),
				signal: AbortSignal.timeout(10000),
			})
		} catch (e) {
			throw new VesperError(0, 'unreachable', `Vesper at ${this.baseUrl} can't be reached (${e.cause?.code || e.message})`)
		}
		const text = await res.text()
		let json = null
		try {
			json = text ? JSON.parse(text) : null
		} catch {
			json = null
		}
		if (!res.ok && !(res.status === 202 && json)) {
			const e = json?.error
			throw new VesperError(res.status, e?.code, e?.message)
		}
		return json
	}

	serverInfo() {
		return this.request('GET', '/server')
	}

	/** Pairs with the code Vesper shows: the site key (also kept on the client). */
	async pair(code, clientName) {
		const r = await this.request('POST', '/pair', { code: String(code).trim(), clientName })
		if (!r?.token) throw new VesperError(202, 'pending', 'the code was not accepted: approve "Companion" in Oriel ▸ Site, or type the current code')
		this.key = r.token
		return r.token
	}

	/** Asks for a site admin's approval (no code): the waiting pairing's id. */
	async requestPairing(clientName) {
		const r = await this.request('POST', '/pair', { clientName })
		if (r?.token) {
			this.key = r.token
			return { token: r.token }
		}
		return { pairingId: r?.pairingId }
	}

	/** A waiting pairing: the key once approved (also kept on the client), `null` while it waits. */
	async pairingStatus(pairingId) {
		const r = await this.request('GET', `/pair/${encodeURIComponent(pairingId)}`)
		if (r?.token) {
			this.key = r.token
			return r.token
		}
		return null
	}

	/** Takes a waiting pairing back. */
	async cancelPairing(pairingId) {
		await this.request('DELETE', `/pair/${encodeURIComponent(pairingId)}`).catch(() => {})
	}

	/** Runs a command (`POST /api/v1/commands`). */
	command(cmd) {
		return this.request('POST', '/commands', cmd)
	}

	/** The lists the dropdowns offer. Lists the server doesn't have are empty. */
	async lists() {
		const get = (path) => this.request('GET', path).catch((e) => (e.status === 404 ? [] : Promise.reject(e)))
		const [looks, clearGroups, timers, messages, props, macros, playlists, presentations] = await Promise.all([
			get('/looks'),
			get('/clear-groups'),
			get('/timers'),
			get('/messages'),
			get('/props'),
			get('/macros'),
			get('/playlists'),
			get('/presentations'),
		])
		return {
			looks: named(looks),
			clearGroups: named(clearGroups),
			timers: named(timers),
			messages: named(messages),
			props: named(props),
			macros: named(macros),
			playlists: flattenPlaylists(playlists),
			presentations: named(presentations),
		}
	}

	/** A playlist with its items (`GET /api/v1/playlists/{id}`). */
	playlist(id) {
		return this.request('GET', `/playlists/${encodeURIComponent(id)}`)
	}

	/** Opens the WebSocket and keeps it open (reconnecting) until `close()`. */
	connect() {
		this.closed = false
		this.open()
	}

	open() {
		if (this.closed) return
		const q = new URLSearchParams({ protocolVersion: String(PROTOCOL_VERSION) })
		if (this.key) q.set('token', this.key)
		const url = this.baseUrl.replace(/^http/, 'ws') + '/api/v1/ws?' + q
		const ws = new WebSocket(url)
		this.ws = ws
		// Each connection starts over: a restarted Vesper counts its state revisions from 1 again,
		// so the last connection's revision (and its state) must not hide the new ones.
		this.revision = 0
		this.state = null
		ws.on('open', () => this.emit('connected'))
		ws.on('message', (data, binary) => {
			if (binary) return // preview frames (never subscribed)
			let m
			try {
				m = JSON.parse(String(data))
			} catch {
				return
			}
			this.handle(m)
		})
		ws.on('unexpected-response', (_req, res) => {
			this.emit('refused', res.statusCode)
			ws.terminate()
		})
		ws.on('error', (e) => this.emit('socketError', e))
		ws.on('close', (code) => {
			if (this.ws === ws) this.ws = null
			this.emit('disconnected', code)
			if (!this.closed) this.timer = setTimeout(() => this.open(), this.reconnectMs)
		})
	}

	handle(m) {
		switch (m.type) {
			case 'hello':
				this.emit('hello', m.server)
				break
			case 'state':
				if (typeof m.revision === 'number' && m.revision < this.revision) return
				this.revision = m.revision ?? this.revision
				this.state = m.state
				this.emit('state', m.state)
				break
			case 'timerTick':
				if (this.state) {
					for (const t of m.timers || []) {
						const cur = this.state.timers?.find((x) => x.id === t.id)
						if (cur) Object.assign(cur, t)
					}
					this.emit('timerTick', m.timers || [])
				}
				break
			case 'documentChanged':
				if (LIST_KINDS.has(m.kind)) this.emit('listsChanged', m.kind)
				break
			case 'notice':
				if (m.key === 'lagged') this.emit('listsChanged', 'lagged')
				break
			case 'error':
				this.emit('serverError', m.error)
				break
		}
	}

	close() {
		this.closed = true
		clearTimeout(this.timer)
		if (this.ws) {
			try {
				this.ws.terminate()
			} catch {
				// already closed
			}
			this.ws = null
		}
	}
}

/** `[{id, name}]` from documents (unnamed ones get a placeholder). */
function named(list) {
	return (Array.isArray(list) ? list : []).map((d, i) => ({ id: d.id, name: d.name || `Untitled ${i + 1}` }))
}

/** Playlists from the tree, depth first, named with their folder path ("Services / Sunday"). */
function flattenPlaylists(tree, prefix = '') {
	const out = []
	for (const n of Array.isArray(tree) ? tree : []) {
		const name = prefix ? `${prefix} / ${n.name}` : n.name
		if (n.kind === 'folder') out.push(...flattenPlaylists(n.children, name))
		else out.push({ id: n.id, name, itemCount: n.itemCount ?? 0 })
	}
	return out
}

module.exports = { VesperClient, VesperError, PROTOCOL_VERSION, flattenPlaylists }
