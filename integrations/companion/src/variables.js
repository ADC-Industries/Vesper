// The module's variables (`$(vesper:<id>)`), from the show state and the lists.

/** A variable id from a name: lowercase letters, digits and `_` ("Walk-in" → "walk_in"). */
function slug(name) {
	return (
		String(name || '')
			.normalize('NFKD')
			.replace(/[̀-ͯ]/g, '')
			.toLowerCase()
			.replace(/[^a-z0-9]+/g, '_')
			.replace(/^_+|_+$/g, '') || 'unnamed'
	)
}

/** Timer variable ids, unique per timer (a second "Countdown" gets `countdown_2`). */
function timerIds(timers) {
	const seen = new Map()
	return (timers || []).map((t) => {
		const base = `timer_${slug(t.name)}`
		const n = (seen.get(base) || 0) + 1
		seen.set(base, n)
		return { id: t.id, name: t.name, varId: n === 1 ? base : `${base}_${n}` }
	})
}

const FIXED = [
	['connection', 'Connection (connected / disconnected)'],
	['server_name', 'Server name'],
	['presentation', 'Live presentation'],
	['slide_number', 'Live slide number (1 = first)'],
	['section', 'Live section (group)'],
	['next_presentation', 'Next slide: presentation'],
	['next_slide_number', 'Next slide: number'],
	['next_section', 'Next slide: section'],
	['look', 'Live look'],
	['media', 'Background or foreground video / image'],
	['audio', 'Audio layer'],
	['transport_position', 'Transport position (m:ss)'],
	['transport_remaining', 'Transport time left (m:ss)'],
	['messages', 'Messages shown'],
	['stage_message', 'Stage message'],
]

/** Variable definitions for the current lists. */
function variableDefinitions(lists) {
	const defs = FIXED.map(([variableId, name]) => ({ variableId, name }))
	for (const t of timerIds(lists?.timers)) {
		defs.push({ variableId: t.varId, name: `Timer ${t.name}: value` })
		defs.push({ variableId: `${t.varId}_state`, name: `Timer ${t.name}: state` })
	}
	return defs
}

function clock(seconds) {
	if (typeof seconds !== 'number' || !Number.isFinite(seconds)) return ''
	const s = Math.max(0, Math.floor(seconds))
	const h = Math.floor(s / 3600)
	const m = Math.floor((s % 3600) / 60)
	const ss = String(s % 60).padStart(2, '0')
	return h ? `${h}:${String(m).padStart(2, '0')}:${ss}` : `${m}:${ss}`
}

/** Variable values from the state (`null` = not connected). */
function variableValues(state, lists, info) {
	const v = {
		connection: state ? 'connected' : 'disconnected',
		server_name: info?.name ?? '',
	}
	const l = state?.layers || {}
	const pos = l.slide
	v.presentation = pos?.presentationName ?? ''
	v.slide_number = pos ? pos.slideIndex + 1 : ''
	v.section = pos?.sectionName ?? ''
	const cur = state?.cursor
	v.next_presentation = cur?.presentationName ?? ''
	v.next_slide_number = cur ? cur.slideIndex + 1 : ''
	v.next_section = cur?.sectionName ?? ''
	const look = state?.activeLook ? (lists?.looks || []).find((x) => x.id === state.activeLook) : null
	v.look = state?.activeLook ? (look?.name ?? '') : state ? 'Every layer' : ''
	v.media = (l.foreground || l.media)?.name ?? ''
	v.audio = l.audio?.name ?? ''
	const tr = state?.transport
	v.transport_position = tr ? clock(tr.position) : ''
	v.transport_remaining = tr && typeof tr.duration === 'number' ? clock(tr.duration - tr.position) : ''
	v.messages = (l.messages || []).map((m) => m.name).join(', ')
	v.stage_message = state?.stageMessage?.text ?? ''
	for (const t of timerIds(lists?.timers)) {
		const live = state?.timers?.find((x) => x.id === t.id)
		v[t.varId] = live?.display ?? ''
		v[`${t.varId}_state`] = live?.state ?? ''
	}
	return v
}

module.exports = { slug, timerIds, variableDefinitions, variableValues, clock }
