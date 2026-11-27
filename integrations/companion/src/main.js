// Bitfocus Companion module for Vesper: actions, feedbacks, variables and presets over Vesper's
// own API (REST commands, the WebSocket's show state). See ../README.md.

const { InstanceBase, InstanceStatus, runEntrypoint } = require('@companion-module/base')
const { VesperClient } = require('./client')
const { buildActions } = require('./actions')
const { buildFeedbacks } = require('./feedbacks')
const { variableDefinitions, variableValues } = require('./variables')
const { buildPresets } = require('./presets')
const { configFields, isLocalHost } = require('./config')

const EMPTY_LISTS = {
	looks: [],
	clearGroups: [],
	timers: [],
	messages: [],
	props: [],
	macros: [],
	playlists: [],
	presentations: [],
}

class VesperInstance extends InstanceBase {
	async init(config, _isFirstInit, secrets) {
		this.config = config || {}
		this.secrets = secrets || {}
		this.lists = EMPTY_LISTS
		this.connected = false
		this.info = null
		this.define()
		await this.start()
	}

	async destroy() {
		this.stop()
	}

	async configUpdated(config, secrets) {
		this.stop()
		this.config = config || {}
		this.secrets = secrets || {}
		await this.start()
	}

	getConfigFields() {
		return configFields()
	}

	/** (Re)defines actions, feedbacks, variables and presets from the lists. */
	define() {
		this.setActionDefinitions(buildActions(this))
		this.setFeedbackDefinitions(buildFeedbacks(this))
		this.setVariableDefinitions(variableDefinitions(this.lists))
		this.setPresetDefinitions(buildPresets(this.lists))
		this.updateVariables()
	}

	updateVariables() {
		this.setVariableValues(variableValues(this.connected ? this.client?.state : null, this.lists, this.info))
	}

	stop() {
		clearTimeout(this.retry)
		clearTimeout(this.listsDue)
		this.client?.close()
		this.client = null
		this.connected = false
	}

	/** Tries again in a few seconds (Vesper not up yet, a network problem). */
	later() {
		clearTimeout(this.retry)
		this.retry = setTimeout(() => void this.start(), 5000)
	}

	async start() {
		const host = (this.config.host || '').trim()
		if (!host) {
			this.updateStatus(InstanceStatus.BadConfig, "Enter Vesper's address")
			return
		}
		const client = new VesperClient({ host, port: this.config.port, key: this.secrets.key })
		this.client = client
		this.updateStatus(InstanceStatus.Connecting)
		try {
			const code = String(this.config.code || '').trim()
			if (code) {
				const key = await client.pair(code, this.config.name || 'Companion')
				this.log('info', 'Paired with Vesper; the key is kept in the secrets store')
				this.secrets = { ...this.secrets, key }
				this.config = { ...this.config, code: '' }
				this.saveConfig(this.config, this.secrets)
			}
			this.info = await client.serverInfo()
			if (this.info.auth === 'unpaired') {
				await this.awaitApproval(client, host)
				return
			}
			await this.refreshLists()
		} catch (e) {
			if (this.client !== client) return
			if (e.status === 401 || e.status === 403) {
				this.updateStatus(InstanceStatus.AuthenticationFailure, e.message)
				return
			}
			this.updateStatus(InstanceStatus.ConnectionFailure, e.message)
			this.later()
			return
		}
		if (this.client !== client) return
		let opened = false
		client.on('connected', () => {
			// After a reconnect the lists may have changed meanwhile (a restarted Vesper, another
			// active collection): fetch them again.
			if (opened) void this.refreshLists().catch((e) => this.log('warn', e.message))
			opened = true
			this.connected = true
			this.updateStatus(InstanceStatus.Ok)
			this.updateVariables()
			this.checkFeedbacks()
		})
		client.on('disconnected', () => {
			if (!this.connected) return
			this.connected = false
			this.updateStatus(InstanceStatus.Disconnected, 'Reconnecting to Vesper')
			this.updateVariables()
			this.checkFeedbacks()
		})
		client.on('refused', (status) => {
			if (status === 401 || status === 403) {
				this.updateStatus(InstanceStatus.AuthenticationFailure, 'Vesper refused the key: pair again')
			}
		})
		client.on('state', () => {
			this.updateVariables()
			this.checkFeedbacks()
		})
		client.on('timerTick', () => {
			this.updateVariables()
			this.checkFeedbacks('timer_running')
		})
		client.on('listsChanged', () => {
			clearTimeout(this.listsDue)
			this.listsDue = setTimeout(() => void this.refreshLists().catch((e) => this.log('warn', e.message)), 500)
		})
		client.connect()
	}

	/**
	 * Not paired and no code: asks for a site admin's approval (Oriel ▸ Site ▸ Join requests)
	 * and waits for it, then starts again with the key.
	 */
	async awaitApproval(client, host) {
		const name = this.config.name || 'Companion'
		const where = isLocalHost(host) ? ' (this machine is not trusted: Vesper runs with --no-trust-localhost)' : ''
		const r = await client.requestPairing(name)
		if (r.token) return this.paired(r.token)
		this.updateStatus(
			InstanceStatus.AuthenticationFailure,
			`Waiting for approval: approve "${name}" in Oriel ▸ Site ▸ Join requests, or type the pairing code${where}`,
		)
		this.pairingId = r.pairingId
		const poll = async () => {
			if (this.client !== client) return client.cancelPairing(r.pairingId)
			try {
				const key = await client.pairingStatus(r.pairingId)
				if (key) return this.paired(key)
				this.retry = setTimeout(() => void poll(), 2000)
			} catch (e) {
				if (this.client !== client) return
				if (e.status === 403) {
					this.updateStatus(InstanceStatus.AuthenticationFailure, `Vesper refused the pairing: ${e.message}`)
				} else {
					// Ended unanswered (or Vesper restarted): ask again.
					this.updateStatus(InstanceStatus.ConnectionFailure, e.message)
					this.later()
				}
			}
		}
		this.retry = setTimeout(() => void poll(), 2000)
	}

	/** Keeps the key from a pairing and starts again with it. */
	async paired(key) {
		this.log('info', 'Paired with Vesper; the key is kept in the secrets store')
		this.secrets = { ...this.secrets, key }
		this.config = { ...this.config, code: '' }
		this.saveConfig(this.config, this.secrets)
		this.stop()
		await this.start()
	}

	async refreshLists() {
		if (!this.client) return
		this.lists = await this.client.lists()
		this.define()
		this.checkFeedbacks()
	}

	/** Sends a command; a refusal is logged (Companion shows the log). */
	async run(cmd) {
		if (!this.client) throw new Error('not connected to Vesper')
		try {
			await this.client.command(cmd)
		} catch (e) {
			this.log('warn', `${cmd.type}: ${e.message}`)
		}
	}
}

runEntrypoint(VesperInstance, [])
