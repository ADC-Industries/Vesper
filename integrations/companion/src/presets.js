// Ready-made buttons: slides, clear, a button per look and per timer.

const { LIVE_STYLE, ON_STYLE, rgb } = require('./feedbacks')
const { timerIds } = require('./variables')

const BLACK = rgb(0, 0, 0)
const WHITE = rgb(255, 255, 255)

function button(category, name, text, down, feedbacks = [], size = '14') {
	return {
		type: 'button',
		category,
		name,
		style: { text, size, color: WHITE, bgcolor: BLACK },
		steps: [{ down, up: [] }],
		feedbacks,
	}
}

function buildPresets(lists) {
	const p = {
		next: button('Slides', 'Next slide', 'NEXT', [{ actionId: 'next', options: {} }]),
		previous: button('Slides', 'Previous slide', 'PREV', [{ actionId: 'previous', options: {} }]),
		live: button('Slides', 'Live slide', '$(vesper:presentation)\\n$(vesper:slide_number)', [], [], 'auto'),
		clear_all: button('Clear', 'Clear all', 'CLEAR ALL', [{ actionId: 'clear_all', options: {} }]),
		clear_slide: button(
			'Clear',
			'Clear the slide',
			'CLEAR SLIDE',
			[{ actionId: 'clear_layer', options: { layer: 'slide' } }],
			[{ feedbackId: 'layer_active', options: { layer: 'slide' }, style: LIVE_STYLE }],
		),
		clear_media: button(
			'Clear',
			'Clear the media',
			'CLEAR MEDIA',
			[{ actionId: 'clear_layer', options: { layer: 'media' } }],
			[{ feedbackId: 'layer_active', options: { layer: 'media' }, style: LIVE_STYLE }],
		),
	}
	for (const l of lists?.looks || []) {
		p[`look_${l.id}`] = button(
			'Looks',
			l.name,
			l.name,
			[{ actionId: 'set_look', options: { look: l.id } }],
			[{ feedbackId: 'look_active', options: { look: l.id }, style: ON_STYLE }],
		)
	}
	const ids = new Map(timerIds(lists?.timers).map((t) => [t.id, t.varId]))
	for (const t of lists?.timers || []) {
		p[`timer_${t.id}`] = button(
			'Timers',
			`${t.name} start / stop`,
			`${t.name}\\n$(vesper:${ids.get(t.id)})`,
			[{ actionId: 'timer', options: { timer: t.id, op: 'toggle' } }],
			[{ feedbackId: 'timer_running', options: { timer: t.id, state: 'any' }, style: ON_STYLE }],
			'auto',
		)
	}
	for (const m of lists?.macros || []) {
		p[`macro_${m.id}`] = button('Macros', m.name, m.name, [{ actionId: 'macro', options: { macro: m.id } }])
	}
	return p
}

module.exports = { buildPresets }
