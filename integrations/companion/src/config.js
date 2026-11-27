// The module's connection settings. The key is kept in Companion's secrets store, never in the
// exported configuration.

const { Regex } = require('@companion-module/base')

function configFields() {
	return [
		{
			type: 'static-text',
			id: 'info',
			width: 12,
			label: 'Vesper',
			value:
				'Vesper\'s address and API port (5555). On the Vesper machine itself (address 127.0.0.1) no pairing is needed. From another computer, either approve "Companion" in Oriel ▸ Site ▸ Join requests (the module asks when it has no key), or type the pairing code Vesper shows (in the server\'s log): the module pairs once, keeps the site key and clears the code.',
		},
		{ type: 'textinput', id: 'host', label: 'Address', width: 8, default: '127.0.0.1', regex: Regex.HOSTNAME },
		{ type: 'number', id: 'port', label: 'Port', width: 4, default: 5555, min: 1, max: 65535 },
		{ type: 'textinput', id: 'code', label: 'Pairing code (used once)', width: 6, default: '' },
		{ type: 'textinput', id: 'name', label: 'Name in the site', width: 6, default: 'Companion' },
		{ type: 'secret-text', id: 'key', label: 'Site key (filled in by pairing, or paste one)', width: 12, default: '' },
	]
}

function isLocalHost(host) {
	return ['127.0.0.1', 'localhost', '::1', '[::1]'].includes(String(host).trim().toLowerCase())
}

module.exports = { configFields, isLocalHost }
