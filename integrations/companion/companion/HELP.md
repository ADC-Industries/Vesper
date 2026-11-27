## Vesper

Controls a Vesper church presentation server over its own API.

**Connection.** Enter Vesper's address and API port (5555). Companion on the Vesper machine
itself (address `127.0.0.1`) needs no pairing. From another computer the module asks Vesper to
pair: approve "Companion" in Oriel ▸ Site ▸ Join requests. Or type the pairing code from the
server's log into **Pairing code** and save. Either way the module pairs once, keeps the site key
in Companion's secrets and clears the code. The key works on every Vesper server of the site;
revoke it in Oriel ▸ Site ▸ Security ▸ Paired computers.

**Actions.** Next / previous slide, trigger a slide by number, trigger a playlist item, clear a
layer / everything / a clear group, bring back a cleared layer, make a look live, start / stop /
reset / adjust timers, show and hide messages (with token values), show / hide / toggle props,
run macros, media transport, audio playlist next / previous, master mute and level, stage
messages.

**Feedbacks.** Slide is live, presentation is live, layer has content, look is live, timer
state, message shown, prop shown, master muted, stage message up, connected.

**Variables.** `$(vesper:presentation)`, `$(vesper:slide_number)`, `$(vesper:section)`, the next
slide's, `$(vesper:look)`, `$(vesper:media)`, `$(vesper:audio)`, the transport's position and
time left, messages, the stage message, and every timer's value and state
(`$(vesper:timer_<name>)`, `$(vesper:timer_<name>_state)`).

Lists (looks, timers, messages, props, macros, playlists, presentations) follow Vesper's edits.
