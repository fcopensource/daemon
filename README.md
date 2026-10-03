# DAEMON

**A sci-fi hacker terminal and system monitor written in Rust.**
Inspired by [eDEX-UI](https://github.com/GitSquared/edex-ui), but native: it has no Electron or
browser engine, and the binary is about 8 MB.

```
┌──────────────┬──────────────────────────────────────┬──────────────┐
│ CLOCK        │ ROOT SHELL (real PTY)                │ ROTATING     │
│ SYSTEM INFO  │                                      │ GLOBE        │
│ CPU GRAPH    │                                      │ NETWORK      │
│ MEMORY GRID  ├──────────────────────────────────────┤ STORAGE      │
│ PROCESSES    │ FILESYSTEM (click dir → cd)          │ CONTROLS     │
└──────────────┴──────────────────────────────────────┴──────────────┘
```

## Features

- **Boot sequence**: Matrix-style digital rain, a fake intrusion log and a glitching
  `ACCESS GRANTED` banner. Press any key to skip it.
- **Working terminal**: runs your shell (PowerShell on Windows, `$SHELL` elsewhere) through a PTY
  (ConPTY on Windows). It supports xterm-256color and truecolor, and has 5000 lines of scrollback.
- **Rotating 3D globe**: a wireframe Earth with dotted continents, pulsing network nodes and
  data packets moving along great-circle links.
- **Hacker sounds**: keystroke clicks, an Enter thunk, boot beeps, an access-granted fanfare and
  ambient data chatter. All sounds are synthesized at runtime, so there are no audio files.
- **Live system stats**: CPU (total and per-core), a memory dot grid, the top processes,
  network RX/TX and disk usage.
- **CRT effects**: scanlines, a moving refresh band and glitch text.
- **File browser**: click a folder to `cd` the shell into it. Click a file to type its path at the prompt.
- **Themes**: `daemon` (neon green, the default), `amber`, `ice`, `red`, `purple`, `tron` and `blade`.

## Run

```sh
cargo run --release
cargo run --release -- --theme amber --fullscreen
cargo run --release -- --mute
```

| Key | Action |
|-----|--------|
| **F10** | toggle sound |
| **F11** | toggle fullscreen |
| Mouse wheel over terminal | scrollback |
| Everything else | goes to the shell |

| Environment variable | Meaning |
|----------------------|---------|
| `DAEMON_THEME` | theme name (same as `--theme`) |
| `DAEMON_SHELL` | shell to launch, e.g. `cmd.exe`, `pwsh`, `zsh` |
| `DAEMON_MUTE`  | start muted when set |

## Building on Windows

Install Rust from https://rustup.rs, then choose either:

- the **MSVC** toolchain (default), which needs *Visual Studio C++ Build Tools*, or
- the **GNU** toolchain without admin rights:
  `rustup-init.exe --default-host x86_64-pc-windows-gnu` plus MinGW
  (`winget install BrechtSanders.WinLibs.POSIX.UCRT`).

On Linux you also need the ALSA headers for sound (`sudo apt install libasound2-dev`).

## Architecture

| File              | Responsibility |
|-------------------|----------------|
| `src/main.rs`     | CLI flags, window setup (`eframe`) |
| `src/app.rs`      | Panel layout, boot sequence, input routing, sound triggers |
| `src/terminal.rs` | PTY spawn (`portable-pty`), output parsing (`vt100`), grid rendering, key → byte mapping |
| `src/globe.rs`    | Orthographic 3D globe: graticule, land dots, arcs, nodes |
| `src/sound.rs`    | Sound synthesizer on its own audio thread (`rodio`) |
| `src/stats.rs`    | Background thread that samples `sysinfo` once a second |
| `src/files.rs`    | Directory listing and clickable tiles |
| `src/widgets.rs`  | Graphs, bars, memory grid, scanlines, glitch text, digital rain |
| `src/theme.rs`    | Color themes |

The PTY reader, the stats sampler and the audio player each run on their own thread. The UI
thread only paints shared state, so slow I/O never stalls rendering.

## Roadmap

- On-screen keyboard
- Multiple terminal tabs
- Following the shell's real working directory in the file browser
- Text selection and copy in the terminal
- JSON theme files

## License

GPL-3.0
