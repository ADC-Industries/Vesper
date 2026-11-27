// The module's actions: each one is a Vesper command (see "Commands" in the protocol README).
// `self` is the module instance (or a test double): `self.lists` holds the dropdowns' lists,
// `self.run(cmd)` sends a command, `self.client` is the `VesperClient`.

const LAYERS = [
	{ id: 'slide', label: 'Slide' },
	{ id: 'media', label: 'Media (background)' },
	{ id: 'announcements', label: 'Announcements' },
	{ id: 'props', label: 'Props' },
	{ id: 'messages', label: 'Messages' },
	{ id: 'audio', label: 'Audio' },
	{ id: 'videoInput', label: 'Video input' },
]

const TRANSPORT_LAYERS = [
	{ id: '', label: 'What plays (foreground, background, then audio)' },
	{ id: 'slide', label: 'Foreground video' },
	{ id: 'media', label: 'Background video' },
	{ id: 'audio', label: 'Audio' },
]

const LIVE = 'live'

/** Dropdown choices from a list (`[{id, name}]`), with `extra` choices first. */
function choices(list, extra = []) {
	return [...extra, ...(list || []).map((d) => ({ id: d.id, label: d.name }))]
}

function dropdown(id, label, list, extra) {
	const c = choices(list, extra)
	return { type: 'dropdown', id, label, choices: c, default: c[0]?.id ?? '', allowCustom: true }
}

/** A 1-based number option. */
function number(id, label, def = 1, min = 1, max = 999) {
	return { type: 'number', id, label, default: def, min, max }
}

/** The action definitions for the current lists. */
function buildActions(self) {
	const l = self.lists || {}
	const run = (cmd) => self.run(cmd)
	const simple = (name, type) => ({ name, options: [], callback: () => run({ type }) })
	const byId = (name, type, field, label, list) => ({
		name,
		options: [dropdown(field, label, list)],
		callback: (e) => (e.options[field] ? run({ type, [field]: e.options[field] }) : undefined),
	})
	return {
		next: simple('Slide: next', 'next'),
		previous: simple('Slide: previous', 'prev'),
		trigger_slide: {
			name: 'Slide: trigger by number',
			options: [
				dropdown('presentation', 'Presentation', l.presentations, [{ id: LIVE, label: 'The live presentation' }]),
				number('slide', 'Slide number (1 = first)'),
			],
			callback: async (e) => {
				const index = Math.max(0, Math.round(Number(e.options.slide) || 1) - 1)
				let presentation = e.options.presentation
				let playlistItem
				if (presentation === LIVE || !presentation) {
					const pos = self.client.state?.layers?.slide ?? self.client.state?.cursor
					if (!pos) throw new Error('no presentation is live')
					presentation = pos.presentation
					if (pos.playlist) playlistItem = { playlist: pos.playlist.playlist, item: pos.playlist.item }
				}
				await run({ type: 'triggerSlide', presentation, index, ...(playlistItem ? { playlistItem } : {}) })
			},
		},
		trigger_playlist_item: {
			name: 'Playlist: trigger item',
			options: [dropdown('playlist', 'Playlist', l.playlists), number('item', 'Item number (1 = first)')],
			callback: async (e) => {
				if (!e.options.playlist) return
				const p = await self.client.playlist(e.options.playlist)
				const item = p.items[Math.round(Number(e.options.item) || 1) - 1]
				if (!item) throw new Error(`${p.name} has no item ${e.options.item}`)
				await run({ type: 'triggerPlaylistItem', playlist: p.id, item: item.id })
			},
		},
		clear_layer: {
			name: 'Clear: layer',
			options: [{ type: 'dropdown', id: 'layer', label: 'Layer', choices: LAYERS, default: 'slide' }],
			callback: (e) => run({ type: 'clearLayer', layer: e.options.layer }),
		},
		restore_layer: {
			name: 'Clear: bring back a cleared layer',
			options: [{ type: 'dropdown', id: 'layer', label: 'Layer', choices: LAYERS.filter((x) => x.id !== 'videoInput'), default: 'slide' }],
			callback: (e) => run({ type: 'restoreLayer', layer: e.options.layer }),
		},
		clear_all: simple('Clear: all', 'clearAll'),
		clear_group: byId('Clear: run a clear group', 'clearGroup', 'id', 'Clear group', l.clearGroups),
		set_look: {
			name: 'Look: make live',
			options: [dropdown('look', 'Look', l.looks, [{ id: '', label: 'Every layer everywhere (no look)' }])],
			callback: (e) => run({ type: 'setLook', look: e.options.look || null }),
		},
		timer: {
			name: 'Timer: start / stop / reset',
			options: [
				dropdown('timer', 'Timer', l.timers),
				{
					type: 'dropdown',
					id: 'op',
					label: 'Operation',
					default: 'timerStart',
					choices: [
						{ id: 'timerStart', label: 'Start' },
						{ id: 'timerStop', label: 'Stop' },
						{ id: 'timerReset', label: 'Reset' },
						{ id: 'toggle', label: 'Start / stop' },
					],
				},
			],
			callback: (e) => {
				const timer = e.options.timer
				if (!timer) return
				let type = e.options.op
				if (type === 'toggle') {
					const t = self.client.state?.timers?.find((x) => x.id === timer)
					type = t && (t.state === 'running' || t.state === 'overrun') ? 'timerStop' : 'timerStart'
				}
				return run({ type, timer })
			},
		},
		timer_adjust: {
			name: 'Timer: add or take away time',
			options: [dropdown('timer', 'Timer', l.timers), number('seconds', 'Seconds (negative takes away)', 60, -35999, 35999)],
			callback: (e) => run({ type: 'timerAdjust', timer: e.options.timer, deltaSeconds: Number(e.options.seconds) || 0 }),
		},
		message_show: {
			name: 'Message: show',
			options: [
				dropdown('message', 'Message', l.messages),
				{
					type: 'textinput',
					id: 'tokens',
					label: 'Token values (name=value; one per line or separated by ;)',
					default: '',
					useVariables: true,
				},
			],
			callback: async (e, ctx) => {
				if (!e.options.message) return
				const raw = ctx?.parseVariablesInString ? await ctx.parseVariablesInString(e.options.tokens || '') : e.options.tokens || ''
				await run({ type: 'messageShow', message: e.options.message, tokens: parseTokens(raw) })
			},
		},
		message_hide: {
			name: 'Message: hide',
			options: [dropdown('message', 'Message', l.messages, [{ id: '', label: 'Every message' }])],
			callback: (e) => run(e.options.message ? { type: 'messageHide', message: e.options.message } : { type: 'messageHide' }),
		},
		prop: {
			name: 'Prop: show / hide / toggle',
			options: [
				dropdown('prop', 'Prop', l.props),
				{
					type: 'dropdown',
					id: 'op',
					label: 'Operation',
					default: 'propToggle',
					choices: [
						{ id: 'propShow', label: 'Show' },
						{ id: 'propHide', label: 'Hide' },
						{ id: 'propToggle', label: 'Toggle' },
					],
				},
			],
			callback: (e) => (e.options.prop ? run({ type: e.options.op, prop: e.options.prop }) : undefined),
		},
		macro: {
			name: 'Macro: run',
			options: [dropdown('macro', 'Macro', l.macros)],
			callback: (e) => (e.options.macro ? run({ type: 'triggerMacro', macro: e.options.macro }) : undefined),
		},
		transport: {
			name: 'Media: transport',
			options: [
				{
					type: 'dropdown',
					id: 'op',
					label: 'Operation',
					default: 'mediaToggle',
					choices: [
						{ id: 'mediaPlay', label: 'Play' },
						{ id: 'mediaPause', label: 'Pause' },
						{ id: 'mediaToggle', label: 'Play / pause' },
						{ id: 'mediaRestart', label: 'Restart' },
						{ id: 'mediaStop', label: 'Stop (clears its layer)' },
					],
				},
				{ type: 'dropdown', id: 'layer', label: 'Layer', choices: TRANSPORT_LAYERS, default: '' },
			],
			callback: (e) => run(e.options.layer ? { type: e.options.op, layer: e.options.layer } : { type: e.options.op }),
		},
		audio_next: simple('Audio: next in the audio playlist', 'audioNext'),
		audio_previous: simple('Audio: previous in the audio playlist', 'audioPrev'),
		master_mute: {
			name: 'Audio: mute master',
			options: [
				{
					type: 'dropdown',
					id: 'mode',
					label: 'Mute',
					default: 'toggle',
					choices: [
						{ id: 'on', label: 'Mute' },
						{ id: 'off', label: 'Unmute' },
						{ id: 'toggle', label: 'Toggle' },
					],
				},
			],
			callback: (e) => {
				const now = !!self.client.state?.audio?.muted
				const muted = e.options.mode === 'toggle' ? !now : e.options.mode === 'on'
				return run({ type: 'setMute', target: 'master', muted })
			},
		},
		master_volume: {
			name: 'Audio: master level',
			options: [number('db', 'Level (dB, −96 … +12)', 0, -96, 12)],
			callback: (e) => run({ type: 'setVolume', target: 'master', db: Number(e.options.db) || 0 }),
		},
		stage_message: {
			name: 'Stage: message',
			options: [
				{ type: 'textinput', id: 'text', label: 'Text', default: '', useVariables: true },
				{ type: 'checkbox', id: 'flash', label: 'Flash', default: false },
			],
			callback: async (e, ctx) => {
				const text = ctx?.parseVariablesInString ? await ctx.parseVariablesInString(e.options.text || '') : e.options.text || ''
				if (!text.trim()) return run({ type: 'stageMessageClear' })
				return run({ type: 'stageMessage', text, ...(e.options.flash ? { flash: true } : {}) })
			},
		},
		stage_message_clear: simple('Stage: clear the message', 'stageMessageClear'),
	}
}

/** `name=value` pairs, one per line or separated by `;`. */
function parseTokens(raw) {
	return String(raw || '')
		.split(/[\n;]/)
		.map((s) => s.trim())
		.filter((s) => s.includes('='))
		.map((s) => {
			const i = s.indexOf('=')
			return { name: s.slice(0, i).trim(), value: s.slice(i + 1).trim() }
		})
		.filter((t) => t.name)
}

module.exports = { buildActions, parseTokens, LAYERS, LIVE }
