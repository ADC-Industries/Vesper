const { test } = require('node:test')
const assert = require('node:assert/strict')
const { once } = require('node:events')
const { VesperClient } = require('../src/client')
const { startMock } = require('./mock-vesper')

test('pairs with a code, sends the key and follows the show state', async () => {
	const mock = await startMock({ requireKey: true })
	const c = new VesperClient({ host: '127.0.0.1', port: mock.port, reconnectMs: 50 })
	try {
		assert.equal((await c.serverInfo()).auth, 'unpaired')
		await assert.rejects(c.pair('000000', 'Companion'), (e) => e.status === 401)
		assert.equal(await c.pair('123456', 'Companion'), mock.KEY)
		assert.equal((await c.serverInfo()).auth, 'paired')

		await c.command({ type: 'next' })
		assert.deepEqual(mock.commands, [{ type: 'next' }])

		const lists = await c.lists()
		assert.deepEqual(
			lists.playlists.map((p) => p.name),
			['Services / Sunday'],
		)
		assert.equal(lists.looks.length, 2)
		assert.equal(lists.macros[0].name, 'Start service')

		const stated = once(c, 'state')
		c.connect()
		await stated
		assert.equal(c.state.layers.slide.presentationName, 'Morning Light')

		const ticked = once(c, 'timerTick')
		mock.push({ type: 'timerTick', timers: [{ id: mock.IDS.timer, state: 'running', currentSeconds: 286, display: '4:46' }] })
		await ticked
		assert.equal(c.state.timers[0].display, '4:46')

		const changed = once(c, 'listsChanged')
		mock.push({ type: 'documentChanged', kind: 'look', id: mock.IDS.look, change: 'changed' })
		assert.deepEqual(await changed, ['look'])

		// An older revision never replaces a newer state.
		const next = once(c, 'state')
		mock.setState({ ...mock.state, activeLook: null })
		await next
		assert.equal(c.state.activeLook, null)
		c.handle({ type: 'state', revision: 0, state: { stale: true } })
		assert.equal(c.state.stale, undefined)
	} finally {
		c.close()
		await mock.close()
	}
})

test('asks for approval without a code and gets the key once approved', async () => {
	const mock = await startMock({ requireKey: true })
	const c = new VesperClient({ host: '127.0.0.1', port: mock.port })
	try {
		const r = await c.requestPairing('Companion')
		assert.equal(r.pairingId, 'p1')
		assert.equal(await c.pairingStatus('p1'), null)
		mock.approve()
		assert.equal(await c.pairingStatus('p1'), mock.KEY)
		assert.equal(c.key, mock.KEY)
	} finally {
		await mock.close()
	}
})

test('a socket without a key is refused', async () => {
	const mock = await startMock({ requireKey: true })
	const c = new VesperClient({ host: '127.0.0.1', port: mock.port, reconnectMs: 10000 })
	try {
		const refused = once(c, 'refused')
		c.connect()
		assert.deepEqual(await refused, [401])
	} finally {
		c.close()
		await mock.close()
	}
})

test('an unreachable server is an error, not a crash', async () => {
	const c = new VesperClient({ host: '127.0.0.1', port: 1 })
	await assert.rejects(c.serverInfo(), (e) => e.code === 'unreachable')
})

test('after Vesper restarts, its new states are followed although their revisions start again at 1', { timeout: 5000 }, async () => {
	const mock = await startMock()
	const c = new VesperClient({ host: '127.0.0.1', port: mock.port, reconnectMs: 20 })
	try {
		const first = once(c, 'state')
		c.connect()
		await first
		for (let i = 0; i < 5; i++) {
			const next = once(c, 'state')
			mock.setState({ ...mock.state, activeLook: null })
			await next
		}
		assert.ok(c.revision >= 6)

		const dropped = once(c, 'disconnected')
		const restarted = { ...mock.state, layers: { ...mock.state.layers, slide: null }, activeLook: mock.IDS.look2 }
		mock.restart(restarted)
		await dropped
		// The first state of the new process (revision 1) replaces the old one.
		await once(c, 'state')
		assert.equal(c.revision, 1)
		assert.equal(c.state.activeLook, mock.IDS.look2)
		assert.equal(c.state.layers.slide, null)
	} finally {
		c.close()
		await mock.close()
	}
})
