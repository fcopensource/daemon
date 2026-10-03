# Custom sounds

Drop audio files in this folder to replace DAEMON's built-in synthesized sounds.
Supported formats: `.wav`, `.ogg`, `.mp3` and `.flac`.

| File name (any supported extension) | When it plays |
|-------------------------------------|---------------|
| `key.wav`      | every keystroke in the terminal |
| `enter.wav`    | the Enter key |
| `boot.wav`     | each line of the boot sequence |
| `granted.wav`  | when the boot sequence completes |
| `click.wav`    | clicking a file or folder |
| `chatter.wav`  | random ambient "data" bursts every few seconds |
| `ambient.mp3`  | **background loop**: plays quietly and repeats forever |

Any sound without a file here keeps its built-in synthesized version. Restart DAEMON
after adding files. The **CONTROLS → CUSTOM SOUNDS** readout shows how many were found.

DAEMON looks for this folder in the current directory, in `assets/sounds`, and next to
the executable (inside `DAEMON.app/Contents/Resources` on macOS).

Short, quiet clips (under 0.2 s) work best for `key` and `enter`.
