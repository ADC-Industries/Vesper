const { test } = require('node:test')
const assert = require('node:assert/strict')
const { buildActions, parseTokens } = require('../src/actions')
const { buildFeedbacks, CHECKS } = require('../src/feedbacks')
const { variableDefinitions, variableValues, slug } = require('../src/variables')
const { buildPresets } = require('../src/presets')
const { baseState, IDS } = require('./mock-vesper')

const LISTS = {
	looks: [
		{ id: IDS.look, name: 'Lyrics' },
		{ id: IDS.look2, name: 'Clean' },
	],
	clearGroups: [],
	timers: [
		{ id: IDS.timer, name: 'Walk-in' },
		{ id: IDS.timer2, name: 'Walk-in' },
	],
	messages: [{ id: IDS.message, name: 'Parking' }],
	props: [{ id: IDS.prop, name: 'Logo' }],
	macros: [{ id: IDS.macro, name: 'Start service' }],
	playlists: [{ id: IDS.playlist, name: 'Services / Sunday' }],
	presentations: [{ id: IDS.song, name: 'Morning Light' }],
}

function fakeSelf(state = baseState()) {
	const sent = []
	return {
		sent,
		lists: LISTS,
		connected: true,
		client: {
			state,
			playlist: async () => ({
				id: IDS.playlist,
				name: 'Sunday',
				items: [{ id: IDS.item0 }, { id: IDS.item1 }],
			}),
		},
		run: async (cmd) => {
			sent.push(cmd)
		},
	}
}

const act = (self, id, options) => buildActions(self)[id].callback({ options }, {})

test('actions send the matching Vesper commands', async () => {
	const self = fakeSelf()
	await act(self, 'next', {})
	await act(self, 'trigger_slide', { presentation: 'live', slide: 4 })
	await act(self, 'trigger_slide', { presentation: IDS.song, slide: 1 })
	await act(self, 'trigger_playlist_item', { playlist: IDS.playlist, item: 2 })
	await act(self, 'set_look', { look: '' })
	await act(self, 'timer', { timer: IDS.timer, op: 'toggle' })
	await act(self, 'timer', { timer: IDS.timer2, op: 'toggle' })
	await act(self, 'message_show', { message: IDS.message, tokens: 'plate=AB 123; colour = blue\nbad' })
	await act(self, 'message_hide', { message: '' })
	await act(self, 'prop', { prop: IDS.prop, op: 'propToggle' })
	await act(self, 'macro', { macro: IDS.macro })
	await act(self, 'master_mute', { mode: 'toggle' })
	await act(self, 'transport', { op: 'mediaToggle', layer: '' })
	await act(self, 'stage_message', { text: '  ', flash: false })
	assert.deepEqual(self.sent, [
		{ type: 'next' },
		{ type: 'triggerSlide', presentation: IDS.song, index: 3, playlistItem: { playlist: IDS.playlist, item: IDS.item1 } },
		{ type: 'triggerSlide', presentation: IDS.song, index: 0 },
		{ type: 'triggerPlaylistItem', playlist: IDS.playlist, item: IDS.item1 },
		{ type: 'setLook', look: null },
		{ type: 'timerStop', timer: IDS.timer },
		{ type: 'timerStart', timer: IDS.timer2 },
		{
			type: 'messageShow',
			message: IDS.message,
			tokens: [
				{ name: 'plate', value: 'AB 123' },
				{ name: 'colour', value: 'blue' },
			],
		},
		{ type: 'messageHide' },
		{ type: 'propToggle', prop: IDS.prop },
		{ type: 'triggerMacro', macro: IDS.macro },
		{ type: 'setMute', target: 'master', muted: true },
		{ type: 'mediaToggle' },
		{ type: 'stageMessageClear' },
	])
	await assert.rejects(act(fakeSelf({ layers: {} }), 'trigger_slide', { presentation: 'live', slide: 1 }), /no presentation is live/)
	assert.deepEqual(parseTokens(''), [])
})

test('feedbacks read the show state', () => {
	const s = baseState()
	assert.equal(CHECKS.slide_live({ presentation: 'live', slide: 3 }, s), true)
	assert.equal(CHECKS.slide_live({ presentation: 'live', slide: 2 }, s), false)
	assert.equal(CHECKS.slide_live({ presentation: IDS.look, slide: 3 }, s), false)
	assert.equal(CHECKS.presentation_live({ presentation: IDS.song }, s), true)
	assert.equal(CHECKS.layer_active({ layer: 'slide' }, s), true)
	assert.equal(CHECKS.layer_active({ layer: 'media' }, s), false)
	assert.equal(CHECKS.layer_active({ layer: 'audio' }, s), true)
	assert.equal(CHECKS.layer_active({ layer: 'props' }, s), false)
	assert.equal(CHECKS.look_active({ look: IDS.look }, s), true)
	assert.equal(CHECKS.look_active({ look: '' }, { ...s, activeLook: null }), true)
	assert.equal(CHECKS.timer_running({ timer: IDS.timer, state: 'any' }, s), true)
	assert.equal(CHECKS.timer_running({ timer: IDS.timer2, state: 'any' }, s), false)
	assert.equal(CHECKS.timer_running({ timer: IDS.timer2, state: 'stopped' }, s), true)
	assert.equal(CHECKS.prop_shown({ prop: IDS.prop }, { layers: { props: [{ prop: IDS.prop }] } }), true)
	assert.equal(CHECKS.master_muted({}, s), false)
	assert.equal(CHECKS.connected({}, null, false), false)
	assert.equal(CHECKS.slide_live({ slide: 1 }, null), false)

	const self = fakeSelf()
	const fb = buildFeedbacks(self)
	assert.equal(fb.slide_live.callback({ options: { presentation: 'live', slide: 3 } }), true)
	assert.equal(fb.look_active.options[0].choices.length, 3)
})

test('variables follow the state, one per timer', () => {
	assert.equal(slug('Walk-in Countdown!'), 'walk_in_countdown')
	assert.equal(slug('Ébène'), 'ebene')
	const ids = variableDefinitions(LISTS).map((d) => d.variableId)
	assert.ok(ids.includes('timer_walk_in') && ids.includes('timer_walk_in_2') && ids.includes('timer_walk_in_2_state'), ids.join())
	assert.equal(new Set(ids).size, ids.length)
	const v = variableValues(baseState(), LISTS, { name: 'Booth' })
	assert.equal(v.connection, 'connected')
	assert.equal(v.server_name, 'Booth')
	assert.equal(v.presentation, 'Morning Light')
	assert.equal(v.slide_number, 3)
	assert.equal(v.section, 'Chorus')
	assert.equal(v.next_slide_number, 4)
	assert.equal(v.look, 'Lyrics')
	assert.equal(v.audio, 'Prelude')
	assert.equal(v.transport_position, '0:31')
	assert.equal(v.transport_remaining, '2:32')
	assert.equal(v.timer_walk_in, '4:47')
	assert.equal(v.timer_walk_in_2_state, 'stopped')
	const off = variableValues(null, LISTS, null)
	assert.equal(off.connection, 'disconnected')
	assert.equal(off.presentation, '')
})

test('presets: slides, clear, one per look, timer and macro', () => {
	const p = buildPresets(LISTS)
	assert.ok(p.next && p.previous && p.clear_all)
	assert.equal(Object.keys(p).filter((k) => k.startsWith('look_')).length, 2)
	assert.equal(Object.keys(p).filter((k) => k.startsWith('timer_')).length, 2)
	assert.equal(p[`timer_${IDS.timer2}`].style.text, 'Walk-in\\n$(vesper:timer_walk_in_2)')
	assert.equal(p[`macro_${IDS.macro}`].steps[0].down[0].actionId, 'macro')
	// Every preset's actions and feedbacks exist.
	const actions = buildActions(fakeSelf())
	const feedbacks = buildFeedbacks(fakeSelf())
	for (const b of Object.values(p)) {
		for (const a of b.steps[0].down) assert.ok(actions[a.actionId], a.actionId)
		for (const f of b.feedbacks) assert.ok(feedbacks[f.feedbackId], f.feedbackId)
	}
})
