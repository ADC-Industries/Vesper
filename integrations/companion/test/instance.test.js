// The module instance (src/main.js) against the mocked Vesper, with a stand-in for Companion's
// `@companion-module/base` (the real one needs the Companion host process).
const { test } = require('node:test')
const assert = require('node:assert/strict')
const { startMock } = require('./mock-vesper')

let Instance = null
class FakeBase {
	constructor() {
		this.statuses = []
		this.variables = {}
		this.saved = null
		this.logs = []
	}
	setActionDefinitions(a) {
		this.actions = a
	}
	setFeedbackDefinitions(f) {
		this.feedbacks = f
	}
	setVariableDefinitions(d) {
		this.variableDefs = d
	}
	setPresetDefinitions(p) {
		this.presets = p
	}
	setVariableValues(v) {
		Object.assign(this.variables, v)
	}
	checkFeedbacks() {}
	updateStatus(status, message) {
		this.statuses.push([status, message ?? null])
	}
	saveConfig(config, secrets) {
		this.saved = { config, secrets }
	}
	log(level, message) {
		this.logs.push([level, message])
	}
}
const base = require.resolve('@companion-module/base')
require.cache[base] = {
	id: base,
	filename: base,
	loaded: true,
	exports: {
		InstanceBase: FakeBase,
		InstanceStatus: {
			Ok: 'ok',
			Connecting: 'connecting',
			Disconnected: 'disconnected',
			ConnectionFailure: 'connection_failure',
			BadConfig: 'bad_config',
			AuthenticationFailure: 'authentication_failure',
		},
		Regex: { HOSTNAME: '' },
		runEntrypoint: (cls) => {
			Instance = cls
		},
	},
}
require('../src/main')

async function until(what, f) {
	for (let i = 0; i < 100; i++) {
		if (f()) return
		await new Promise((r) => setTimeout(r, 50))
	}
	assert.fail(`timed out waiting for ${what}`)
}

test('the instance asks for approval, pairs, follows the state and runs actions', async () => {
	const mock = await startMock({ requireKey: true })
	const inst = new Instance()
	try {
		await inst.init({ host: '127.0.0.1', port: mock.port, code: '', name: 'Companion' }, true, {})
		const last = () => inst.statuses.at(-1)
		assert.equal(last()[0], 'authentication_failure')
		assert.match(last()[1], /Join requests/)
		mock.approve()
		await until('the key', () => inst.saved?.secrets?.key === mock.KEY)
		await until('connected', () => last()[0] === 'ok')
		await until('the state', () => inst.variables.presentation === 'Morning Light')
		assert.equal(inst.variables.server_name, 'Booth')
		assert.equal(inst.variables.look, 'Lyrics')
		assert.ok(inst.variableDefs.some((d) => d.variableId === 'timer_walk_in_2'))
		assert.equal(inst.actions.set_look.options[0].choices.length, 3)

		await inst.actions.next.callback({ options: {} }, {})
		await inst.actions.macro.callback({ options: { macro: mock.IDS.macro } }, {})
		assert.deepEqual(mock.commands, [{ type: 'next' }, { type: 'triggerMacro', macro: mock.IDS.macro }])
		assert.equal(inst.feedbacks.slide_live.callback({ options: { presentation: 'live', slide: 3 } }), true)
		assert.equal(inst.feedbacks.connected.callback({ options: {} }), true)
	} finally {
		await inst.destroy()
		await mock.close()
	}
})

test('a pairing code pairs at once; a bad address is a config problem', async () => {
	const mock = await startMock({ requireKey: true })
	const inst = new Instance()
	try {
		await inst.init({ host: '127.0.0.1', port: mock.port, code: '123456', name: 'Companion' }, true, {})
		assert.equal(inst.saved.secrets.key, mock.KEY)
		assert.equal(inst.saved.config.code, '')
		await until('connected', () => inst.statuses.at(-1)[0] === 'ok')
	} finally {
		await inst.destroy()
		await mock.close()
	}
	const empty = new Instance()
	await empty.init({ host: '', port: 5555 }, true, {})
	assert.equal(empty.statuses.at(-1)[0], 'bad_config')
	await empty.destroy()
})

test('after Vesper restarts, the instance follows the new state and fetches the lists again', async () => {
	const mock = await startMock()
	const inst = new Instance()
	try {
		await inst.init({ host: '127.0.0.1', port: mock.port, code: '', name: 'Companion' }, true, {})
		await until('the state', () => inst.variables.presentation === 'Morning Light')
		for (let i = 0; i < 5; i++) mock.setState({ ...mock.state })
		await until('the revision', () => inst.client.revision >= 6)
		assert.equal(inst.actions.set_look.options[0].choices.length, 3)

		mock.setList('/api/v1/looks', [{ id: mock.IDS.look2, name: 'Clean' }])
		const slide = { ...mock.state.layers.slide, presentationName: 'Evening Song' }
		mock.restart({ ...mock.state, layers: { ...mock.state.layers, slide } })
		await until('the new state', () => inst.variables.presentation === 'Evening Song')
		await until('the new lists', () => inst.actions.set_look.options[0].choices.length === 2)
		assert.equal(inst.statuses.at(-1)[0], 'ok')
	} finally {
		await inst.destroy()
		await mock.close()
	}
})
