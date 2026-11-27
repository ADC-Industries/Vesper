// The module's feedbacks (boolean: a button changes colour while it is true), read from the
// show state the WebSocket sends (`ShowState` in the protocol README).

const { LAYERS, LIVE } = require('./actions')

const LIVE_STYLE = { bgcolor: rgb(200, 40, 30), color: rgb(255, 255, 255) }
const ON_STYLE = { bgcolor: rgb(110, 70, 200), color: rgb(255, 255, 255) }

function rgb(r, g, b) {
	return (r << 16) + (g << 8) + b
}

/** Whether a layer has something on it. */
function layerActive(state, layer) {
	const l = state?.layers
	if (!l) return false
	switch (layer) {
		case 'slide':
			return !!l.slide || !!l.foreground
		case 'props':
		case 'messages':
			return (l[layer] || []).length > 0
		default:
			return !!l[layer]
	}
}

/** The boolean checks, by feedback id: `(options, state, connected) => boolean`. */
const CHECKS = {
	slide_live: (o, s) => {
		const pos = s?.layers?.slide
		if (!pos) return false
		if (o.presentation && o.presentation !== LIVE && o.presentation !== pos.presentation) return false
		return pos.slideIndex === Math.round(Number(o.slide) || 1) - 1
	},
	presentation_live: (o, s) => !!o.presentation && s?.layers?.slide?.presentation === o.presentation,
	layer_active: (o, s) => layerActive(s, o.layer),
	look_active: (o, s) => (s?.activeLook ?? '') === (o.look ?? ''),
	timer_running: (o, s) => {
		const t = s?.timers?.find((x) => x.id === o.timer)
		return !!t && (o.state === 'any' ? t.state === 'running' || t.state === 'overrun' : t.state === o.state)
	},
	message_shown: (o, s) => (s?.layers?.messages || []).some((m) => m.message === o.message),
	prop_shown: (o, s) => (s?.layers?.props || []).some((p) => p.prop === o.prop),
	master_muted: (_o, s) => !!s?.audio?.muted,
	stage_message: (_o, s) => !!s?.stageMessage,
	connected: (_o, _s, connected) => !!connected,
}

function dd(id, label, list, extra = []) {
	const choices = [...extra, ...(list || []).map((d) => ({ id: d.id, label: d.name }))]
	return { type: 'dropdown', id, label, choices, default: choices[0]?.id ?? '', allowCustom: true }
}

/** The feedback definitions for the current lists. `self.client.state`: the show state. */
function buildFeedbacks(self) {
	const l = self.lists || {}
	const def = (name, style, options, id) => ({
		type: 'boolean',
		name,
		defaultStyle: style,
		options,
		callback: (fb) => CHECKS[id](fb.options, self.client?.state, self.connected),
	})
	return {
		slide_live: def(
			'Slide is live',
			LIVE_STYLE,
			[
				dd('presentation', 'Presentation', l.presentations, [{ id: LIVE, label: 'Whichever is live' }]),
				{ type: 'number', id: 'slide', label: 'Slide number (1 = first)', default: 1, min: 1, max: 999 },
			],
			'slide_live',
		),
		presentation_live: def('Presentation is live', LIVE_STYLE, [dd('presentation', 'Presentation', l.presentations)], 'presentation_live'),
		layer_active: def(
			'Layer has content',
			LIVE_STYLE,
			[{ type: 'dropdown', id: 'layer', label: 'Layer', choices: LAYERS, default: 'slide' }],
			'layer_active',
		),
		look_active: def('Look is live', ON_STYLE, [dd('look', 'Look', l.looks, [{ id: '', label: 'No look (every layer)' }])], 'look_active'),
		timer_running: def(
			'Timer state',
			ON_STYLE,
			[
				dd('timer', 'Timer', l.timers),
				{
					type: 'dropdown',
					id: 'state',
					label: 'State',
					default: 'any',
					choices: [
						{ id: 'any', label: 'Counting (running or overrun)' },
						{ id: 'running', label: 'Running' },
						{ id: 'paused', label: 'Paused' },
						{ id: 'overrun', label: 'Overrun' },
						{ id: 'expired', label: 'Expired' },
						{ id: 'stopped', label: 'Stopped' },
					],
				},
			],
			'timer_running',
		),
		message_shown: def('Message is shown', LIVE_STYLE, [dd('message', 'Message', l.messages)], 'message_shown'),
		prop_shown: def('Prop is shown', LIVE_STYLE, [dd('prop', 'Prop', l.props)], 'prop_shown'),
		master_muted: def('Master is muted', LIVE_STYLE, [], 'master_muted'),
		stage_message: def('A stage message is up', ON_STYLE, [], 'stage_message'),
		connected: def('Connected to Vesper', { bgcolor: rgb(0, 120, 60), color: rgb(255, 255, 255) }, [], 'connected'),
	}
}

module.exports = { buildFeedbacks, CHECKS, layerActive, rgb, LIVE_STYLE, ON_STYLE }
