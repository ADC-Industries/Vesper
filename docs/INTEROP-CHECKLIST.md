# Interop checklist: Vesper's IDN outputs and inputs on real gear

Vesper sends and receives network video with **IDN Open Protocol**, an open implementation
compatible with NDI®. The automated tests only check IDN against IDN. Use this list to check
Vesper against the third-party gear on your network, once per receiver or sender, and again
after updating Vesper or the IDN library.

NDI® is a registered trademark of Vizrt NDI AB. Vesper and IDN Open Protocol are not
affiliated with or endorsed by Vizrt.

## Before you start

- Vesper is running on the server and has at least two outputs: one opaque (e.g. Audience)
  and one with alpha (e.g. a lower-thirds key feed), one of them with **Embed audio** on.
  Configure them in Oriel under **Screens ▸ Configure Screens**.
- On the server, `cargo run --release -p output --example idn_probe -- --list` shows what
  is on the network, including Vesper's own sources (`HOST (source name)`).
- Write down: the Vesper version (`vesper --version`), the IDN library version (Oriel shows
  it in the transport list), the receiver's name and version, and the network (same switch,
  VLAN, Wi-Fi or wired).

## Receivers to try

Pick what the church actually uses, plus one more if you can:

- [ ] vMix
- [ ] OBS (with a plugin for network video compatible with NDI®)
- [ ] NDI Studio Monitor or a similar monitor app
- [ ] A hardware switcher or converter (e.g. a vision mixer with network inputs)

## For each receiver

| Check | How | Pass when |
|---|---|---|
| **Discovery** | Open the receiver's source list. | Every Vesper output appears as `HOST (name)` within about 10 s. Note how long it took. |
| **Groups** | Give one output a group in Oriel; set the receiver to that group. | It appears only for receivers in that group. |
| **Video** | Show a slide with fine text, or a test pattern from Configure Screens. | The picture is sharp, colours match the preview in Oriel, nothing is shifted or cropped. |
| **Key / alpha** | Put the key-feed output on a layer above a camera in the receiver. | The background is transparent, text edges are clean (no dark or bright fringe), half-transparent parts look half-transparent. |
| **Audio** | Play a video with sound on the output that embeds audio. | The receiver shows audio meters; the sound is clean, at the expected level, and in sync with the picture (clap or beep video: within a frame). |
| **Frame rate** | Compare the receiver's reported format with the screen's (e.g. 1920×1080 at 29.97 or 30). Watch a moving video for a minute. | The reported rate matches; motion is smooth, with no regular stutter. |
| **Rename** | Rename the output in Oriel and apply. | The old name disappears and the new one appears; the receiver can switch to it. |
| **Reconnect after unplug** | Unplug the server's network cable for 10 s, then plug it back. Repeat by restarting Vesper. | The receiver gets the picture back by itself within about 10 s, without reselecting the source. |
| **Long run** | Leave it running through a rehearsal (an hour or more). | No freezes, drops to black or audio dropouts. |

### Inputs (when the receiving side is used)

- [ ] A source from the receiver's machine (vMix output, OBS output, a camera converter) shows
  up in `idn_probe --list` on the server.
- [ ] `idn_probe "<source>" --frames 90 --png /tmp/in.png` receives it at the right size and
  rate; the PNG looks right.
- [ ] A source with alpha keeps its transparency (`idn_probe` prints the alpha range).
- [ ] `idn_av_probe "<source>" 30` on a clap/beep clip reports offsets within a frame.

## When something fails: what to record

The IDN library is a separate project. Report problems as **black-box notes**: what you did,
what you saw, what you expected. Do not include anything taken from any vendor's SDK,
documentation or tools' internals; just what the products show on screen.

- **Steps:** the exact steps, starting from a fresh start of Vesper and the receiver.
- **Observed:** what happened, with timestamps. A phone photo or screenshot of the
  receiver helps.
- **Expected:** what should have happened.
- **Setup:** Vesper and IDN library versions; receiver product and version; operating systems;
  the screen's size, frame rate and alpha mode; whether the output embeds audio; groups; the
  network path (same switch, VLAN, Wi-Fi).
- **Vesper side:** the server log around the time (`journalctl` or the terminal), and the
  output's status line in Configure Screens (frames sent / dropped / errors).
- **Server measurements**, if you can run them during the problem:
  - `idn_probe --list` (is the source listed?),
  - `idn_pacing "<source>" --seconds 30 --csv pacing.csv` (stalls, repeats, skips),
  - `idn_av_probe "<source>" 30` (A/V offset),
  - CPU load of the `vesper` process (`top`).
- **Does it reproduce** with another receiver, on a wired connection, or with only one output?

## Capacity reminder

The IDN library encodes in software: about **1.8 CPU cores per 1080p30 stream** today. Plan
on about **2 outputs per mini PC** until the library's encoder improvements land. If frames
drop (the output's "dropped" count rises) with more outputs, that is the likely cause; note
the CPU load in the report.
