# KAAMOS

*Kaamos* is the Finnish polar night, the weeks when the sun never rises.
This is a procedural 80s night-synth machine for it: arpeggiated analog synths, Juno pads,
an ominous low drone, gated-reverb drums, FM bells and a wobbly VHS tape stage.
All of it is synthesized in Rust, live, forever. No samples, no plugins.

![KAAMOS running in the terminal](docs/terminal.png)

## Why

I've played keyboards since I was five, but I've never been able to write proper songs.
C64, the demoscene, Scream Tracker, FastTracker, and last year finally learning Reaper properly.

I was kind of vibing all the time and sounded like Ross Geller. So I replicated my way of
working (randomness with highs and lows, with harmonies) and generated this for you.

## What it does

One knob matters most: **intensity**. At 0 it's a dark pad and a drone breathing in the
snow. At 1 it's a full pulsing synth score with drums, sequenced bass and a lead.
Everything in between fades in and out smoothly, so you can ride it during a talk.

The music never repeats on its own: modes, chord progressions, arpeggio motifs, bass lines,
melodies, fills and key changes are generated as it plays. When you hear something you
like, you can keep it (repeat or lock the pattern) and capture it as
[Strudel](https://strudel.cc/) code.

## Run

```sh
cargo run --release
```

macOS needs nothing extra. On Linux (incl. Steam Deck) you need the ALSA headers:
`sudo apt install libasound2-dev pkg-config` (or `pacman -S alsa-lib`).

It starts quietly at intensity 0.3, fades in, and prints a link like
`http://192.168.1.23:7878`. Open it on your phone (same Wi-Fi) for the stage panel.

<img src="docs/phone-panel.png" alt="Phone control panel" width="300">

## Controls

| Key | Action |
|---|---|
| `↑` `↓` | intensity ± 0.05 |
| `1` … `9` | jump intensity: 1 = dormant … 5 = pulse … 9 = peak (it glides, never jumps) |
| `←` `→` | darkness (bright nostalgic modes ↔ dark/phrygian) |
| `,` `.` | tempo ± 2 BPM |
| `[` `]` | space (reverb) |
| `o` `p` | tape / VHS wear |
| `-` `=` | volume |
| `q w e r t y u` | layers drone / pad / arp / bass / drums / lead / bells: cycle **auto → on → off** |
| `SPACE` | **BOOM**: sub drop + crash, for slide reveals (instant) |
| `g` | riser that lands on a bar line |
| `f` | drum fill on the next bar |
| `n` | new section (new progression, arp motif, melody) on the next bar |
| `k` | modulate to a new key |
| `l` | repeat the current pattern one more round (press 3× = 3 extra rounds) |
| `L` | lock the pattern: it repeats until you press `L` again (`n` still jumps on) |
| `c` | capture the current pattern as Strudel code |
| `h` | hold: freeze on the current chord |
| `z` | drift: the machine wanders between calm and intense by itself |
| `x` | fade out (10 s) / fade back in |
| `Esc` / `Q` | quit (with a short fade) |

Intensity stays live while a pattern repeats or is locked: the same chords, motif and
melody keep going while layers, drums and filters follow the knob. Lock a pattern and push
the intensity up for a build.

The phone panel has the same controls plus big preset buttons
(DORMANT · EERIE · PULSE · DRIVE · PEAK), REPEAT, LOCK, CAPTURE and the drift range.

## Capture to Strudel

Press `c` (or CAPTURE on the phone) and the pattern that is playing is written as code to
`kaamos-<seed>.strudel.js` in the current folder (`--export-dir` to change it):
pad chords, arpeggio, bass line, melody, drums, drone and bells, with the filter and
reverb settings of that moment.

Every capture with the same seed is **appended** to the same file as `p1`, `p2`, `p3` …,
and the `arrange(...)` line at the end plays them in order as a song. A new seed starts a
new file. Paste the file into <https://strudel.cc> and press Ctrl+Enter.

Capturing the same pattern at different intensities gives you a quiet and a full
version of it to arrange.

## For a presentation

- Run it on the presenting laptop, control it from the phone: your slides keep focus.
- Turn on **drift** with a range like 0.15 to 0.5 and it scores the whole talk unattended.
- `--seed 1234` gives the same musical material every time (handy for rehearsal).
- `--headless` skips the terminal UI; `--bind 127.0.0.1` keeps the panel local-only.
- Without a network, the panel still works on `http://localhost:7878` on the same machine.

## Offline render (for videos)

```sh
kaamos render score.wav --minutes 4 --seed 7 \
  --curve  "0:0.05, 30:0.25, 1m:0.5, 1m40:0.75, 2m10:1, 3m:0.3" \
  --events "1m58:riser, 2m30:boom, 2m40:lead=off, 3m30:fade"
```

`--curve` automates intensity (`time:value`, times in seconds or `1m30`).
`--events` fires any command at a time: triggers `boom fill riser next key loop export`,
toggles `drift hold lock fade`, layers `arp=off` / `lead=on` / `drums=auto`, or params
`darkness=0.9`, `bpm=100`, `space=0.8` … The render ends with a fade (`--no-fade` to skip).

## Pipe mode

`kaamos --stdout | ffplay -f s16le -ar 48000 -ac 2 -nodisp -` streams raw PCM instead of
using the sound card (builds without the `audio` feature can only do this and `render`).

## Web API

`GET /api/state` returns JSON. `GET /api/cmd?c=<command>` runs the same commands as
`--events` (URL-encode `=` as `%3D`), e.g. `curl "localhost:7878/api/cmd?c=boom"`.
Handy for triggering from Keynote/OBS scripts or a Stream Deck.

## Inside

- `dsp.rs`: polyBLEP oscillators, zero-delay-feedback ladder filter, SVF, envelopes
- `instruments.rs`: Juno pad, resonant arp, sequencer bass, drone + air, glide lead,
  FM bells, 808/Linn-style drums, boom / crash / riser
- `fx.rs`: ensemble chorus, ping-pong tape echo, 8-line FDN hall, gated reverb, tape
  machine (saturation, wow & flutter, hiss), look-ahead limiter
- `music.rs`: modes ordered bright to dark, diatonic progressions, voice-led voicings,
  arp motifs, bass patterns, phrase-based lead melodies
- `engine.rs`: the clock, the composer (sections, key changes, fills, repeats) and the mix;
  intensity drives layer gains, filter openness, arp rate, drum patterns and sidechain pump
- `strudel.rs`: pattern capture to Strudel code

The only dependency is [cpal](https://github.com/RustAudio/cpal) for sound-card output.
Everything else (DSP, terminal UI, web server) is plain `std`.

## Ideas

- MIDI controller input (mod wheel → intensity).
- Record the live session to WAV.

## License

MIT
