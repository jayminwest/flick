# Dictation

The `dictation` module types what you say into any app. Hold a key chord (here `right_cmd+right_shift`), speak, and release. Flick transcribes the audio on this Mac and puts the text into the focused field. A small pill at the bottom of the screen shows a level bar while it records, then `Transcribing`, then the result or an error. Esc while the pill shows cancels.

Everything stays on this Mac. The recorder (sox's `rec`) and the speech-to-text engine (whisper.cpp's `whisper-cli` by default) are local programs. The module has no network code, and peers cannot use any dictation verb.

## Setup

1. **Install the programs.** Homebrew's whisper.cpp has `whisper-cli` and `parakeet-cli`. sox has `rec`.

   ```bash
   brew install whisper-cpp sox
   ```

2. **Download the model.** Flick never downloads models. The default `model` is whisper large-v3-turbo, 5-bit quantized (`ggml-large-v3-turbo-q5_0.bin`, about 574 MB), at the path Flick expects:

   ```bash
   curl -L --fail --create-dirs \
     -o "$HOME/Library/Application Support/Flick/models/ggml-large-v3-turbo-q5_0.bin" \
     https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin
   ```

   For a smaller and faster model, download another `ggml-*.bin` from the same repository (for example `ggml-small.bin` or `ggml-base.en.bin`) and set `model` to its path.

3. **Turn the module on and add the chord.** The module is off until `[dictation]` sets at least one key. Flick cannot tell an empty table from a missing one. The trigger is a `[[keys.chord]]` with `flick` actions ([Key triggers](keys.md)), because modules never import each other:

   ```toml
   [dictation]
   engine = "whisper"

   [[keys.chord]]
   name = "dictate"
   keys = ["right_cmd", "right_shift"]
   on_down = { flick = "dictation start" }
   on_up = { flick = "dictation stop" }
   ```

   Run **Reload Flick Config**. The key tap needs **Accessibility**, as every key trigger does. A config with `[keys]` or a chord turns the key tap on.

4. **Allow the microphone.** The first hold asks for **Microphone** access and records nothing. Flick does this so the first clip is never the silence `rec` would capture while the prompt shows. Click Allow, then hold the chord again. To check or change the grant: **System Settings → Privacy & Security → Microphone → Flick**. macOS gives the grant to Flick.app, because Flick starts `rec`. A Flick.app rebuilt with ad-hoc signing (no Apple Development identity) is a new app to macOS, so the grant must be given again, as for Accessibility.

5. **Check.** `flick dictation status` shows what is missing:

   ```text
   dictation: on
   state: idle
   engine: whisper /opt/homebrew/bin/whisper-cli (found)
   model: /Users/you/Library/Application Support/Flick/models/ggml-large-v3-turbo-q5_0.bin (found)
   recorder: /opt/homebrew/bin/rec (found)
   microphone: authorized
   insert: paste (clipboard restored after 250 ms)
   ```

## Settings

Every key, with its default:

```toml
[dictation]
engine = "whisper"                 # whisper (whisper-cli), parakeet (parakeet-cli) or command
whisper_bin = "/opt/homebrew/bin/whisper-cli"
parakeet_bin = "/opt/homebrew/bin/parakeet-cli"
# command = ["/usr/local/bin/my-stt", "--model", "{model}", "{wav}"]  # engine = "command"; unset by default
model = "~/Library/Application Support/Flick/models/ggml-large-v3-turbo-q5_0.bin"
language = "en"                    # a language code, or "auto"
prompt = ""                        # words to prime whisper with: names, jargon
vad_model = ""                     # a Silero VAD model for whisper-cli --vad; "" for none
recorder = "/opt/homebrew/bin/rec"
max_seconds = 120                  # recording stops by itself after this (at most 600)
min_hold_ms = 300                  # a shorter hold is an accidental press: nothing is recorded
timeout_secs = 30                  # the engine's time budget per clip
silence_rms = 0.01                 # a clip whose loudest 50 ms stays under this is silence
insert = "paste"                   # paste (clipboard and cmd+V, clipboard restored) or type (key events)
restore_ms = 250                   # with paste: when the clipboard is restored
trailing_space = true              # a space after the text, so the next dictation does not run into it
```

- **Engines.** `whisper` runs `whisper-cli -m <model> -f <wav> -l <language> -nt -np`, plus `--prompt <prompt>` and `--vad -vm <vad_model>` when those are set. `parakeet` runs `parakeet-cli -m <model> -f <wav> -np`, with a ggml Parakeet model in `model` (none is the default). `command` runs your own program. `{wav}` (required), `{model}`, `{language}` and `{prompt}` in its arguments are replaced, and the transcript is its stdout.
- **VAD.** A Silero VAD model makes whisper skip non-speech: `curl -L --fail -o "$HOME/Library/Application Support/Flick/models/ggml-silero-v5.1.2.bin" https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v5.1.2.bin`, then set `vad_model` to that path.
- **Insert.** `paste` saves every clipboard item, puts the text on the clipboard (marked transient and concealed, so clipboard history skips it), sends cmd+V, and puts the old clipboard back after `restore_ms`. It restores only if nothing else changed the clipboard meanwhile. `type` sends the text as key events and never touches the clipboard. Use it for apps where paste misbehaves.

```bash
flick dictation status   # on/off, state, engine, model, recorder, microphone, insert, last error
flick dictation start    # what the chord's on_down sends
flick dictation stop     # what its on_up sends
flick dictation cancel   # stop recording or transcribing; insert nothing
flick dictation last     # the last transcript (memory only), also one that was not inserted
```

## What happens

1. **Chord down** (`dictation start`). Flick checks the microphone grant, the recorder, the engine and the model. It starts `rec` (16 kHz mono 16-bit, kept in memory, no file) and shows the pill with a level bar.
2. **Chord up** (`dictation stop`). A hold shorter than `min_hold_ms` is discarded. Otherwise Flick notes the app in front and stops `rec`. A clip with no sound above `silence_rms` is not transcribed (`No speech heard`). This stops whisper's "Thank you." on silence. Otherwise Flick writes the clip to `~/Library/Caches/Flick/dictation/` (folder 0700, file 0600), runs the engine within `timeout_secs`, and deletes the clip on every outcome. Whisper's non-speech tokens (`[BLANK_AUDIO]`, `(music)`, `*laughs*`, `♪`) are removed.
3. **Insert.** Flick waits until the chord's keys are up (at most 500 ms), so cmd+V is not read as cmd+shift+V. It does not insert while secure input is on (a password field) or when another app is in front than at chord-up. The pill then says why, and `flick dictation last` has the text. Otherwise it pastes or types the text.

## Privacy

- Audio stays in memory while recording. The clip file exists only while the engine runs. Leftover clips from a crash are deleted when Flick starts.
- Transcripts and audio are never logged or stored. Log lines carry durations and character counts. `last` is in memory only and is gone when Flick quits.
- No network: the engine and the recorder are local programs, and Flick passes them no URL. Every `dictation` verb is refused over the network (`flick: dictation start: not allowed over the network`), so a peer can never start this Mac's microphone. A card's `flick` action cannot either.
- With `insert = "paste"`, the transcript is on the clipboard for `restore_ms`. It is marked concealed, so clipboard history and well-behaved clipboard managers skip it.

## Limits

- **Latency.** whisper-cli loads the model on every dictation (a cold process). On an M4 Pro, small takes about 0.6 s for 11 s of audio. large-v3-turbo-q5_0 is slower. If release-to-text is too slow, use a smaller `model`. A warm whisper-server engine is not built yet.
- **First syllable.** `rec` takes about 50-150 ms to open the microphone after chord down. Start speaking after the pill shows.
- **The first hold after install records nothing**, because macOS asks for the microphone then.
- **Secure input** blocks insertion, not recording: the text waits in `flick dictation last`.
- **Chord overlap.** The KOTA push-to-talk chord (`right_cmd+right_alt`) shares `right_cmd`. Holding `right_cmd+right_shift+right_alt` fires both chords.
- **No live text.** The text comes after release, not while you speak. One dictation is at most `max_seconds` (600 at most).
- **Apps that remap cmd+V** (some terminals, remote desktops) may not paste. Use `insert = "type"`.

## Manual test checklist

Unit tests cover the session, the recorder and engine runs with fake programs, and the text cleanup. This checklist covers the real microphone, engine, pill and insertion. Run it on the laptop, on the installed Flick.app, after a change to `src/modules/dictation/`, `platform::pill`, `platform::mic`, `keytap::paste` or `pasteboard`. Note the release-to-text time for each engine and model.

- [ ] **Status before setup.** Without `[dictation]`: `flick dictation status` says `dictation: off: set a key in [dictation] to turn it on`, and a `dictation start` says the same.
- [ ] **Missing pieces.** With `[dictation]` on but no model downloaded: status shows `model: ... (missing; Flick never downloads models, see docs/dictation.md)`. Holding the chord shows the pill `Model missing: see flick dictation status`.
- [ ] **Microphone prompt.** On a fresh grant (`tccutil reset Microphone com.jayminwest.flick`), the first hold shows the macOS prompt and the pill says `Allow the microphone, then hold again`. After Allow, status says `microphone: authorized`.
- [ ] **Microphone denied.** Turn Flick off under Privacy & Security → Microphone. A hold shows `Microphone denied`, and status says `denied (...)`. Turn it back on.
- [ ] **TextEdit.** Hold `right_cmd+right_shift` for about 5 s and say a sentence. On release the pill shows `Transcribing`, and the sentence appears at the cursor with a trailing space. Note the time from release to text.
- [ ] **Other apps.** The same in Safari (a web text area), Slack or another Electron app, Ghostty or Terminal, and Flick's own launcher field.
- [ ] **10 s utterance latency.** Speak for about 10 s. Note the release-to-text time with large-v3-turbo-q5_0 (target: at most 1.0 s). Optionally note it with small or base too.
- [ ] **Clipboard kept.** Copy an image (or a password from 1Password), then dictate into TextEdit. Afterwards cmd+V pastes the image (or password), not the transcript. `flick clip list` does not show the transcript.
- [ ] **Short press.** Tap the chord quickly (under 300 ms): nothing is recorded or inserted.
- [ ] **Silence.** Hold for 3 s without speaking: the pill says `No speech heard`, and nothing is inserted.
- [ ] **Esc.** Hold, speak, and press Esc while still holding: the pill hides and nothing is inserted. Do it again with Esc during `Transcribing`: nothing is inserted.
- [ ] **Focus change.** Release the chord and switch apps at once (cmd+Tab): nothing is inserted. The pill says `Another app is in front: kept, see flick dictation last`, and `flick dictation last` prints the text.
- [ ] **Secure input.** Dictate into a password field (or with Terminal's Secure Keyboard Entry on): nothing is inserted, the pill says `Secure input is on`, and `flick dictation last` has the text.
- [ ] **Type mode.** Set `insert = "type"` and reload. Dictation still works in TextEdit, and the clipboard is never touched. Set it back.
- [ ] **No leftovers.** After a normal run, a cancel and an engine error (set `whisper_bin` to `/usr/bin/false`), `ls ~/Library/Caches/Flick/dictation` is empty.
- [ ] **No transcript in logs.** Flick's stderr (the launchd agent's log file, or the terminal when you start Flick by hand) has `flick: dictation: ...` lines with durations and character counts only, never the spoken words.
- [ ] **Network refusal.** From mbp-server: `flick --host jaymins-macbook-pro dictation start` prints `flick: dictation start: not allowed over the network` and exits 1. The laptop's microphone indicator does not light.
- [ ] **Chord overlap.** Hold `right_cmd+right_shift+right_alt`: note whether KOTA push-to-talk fires too (it is expected to).
