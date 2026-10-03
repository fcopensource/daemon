<div align="center">

<img src="assets/icon.png" width="96" alt="DAEMON icon">

# Running DAEMON on macOS

Works on Apple Silicon (M1–M4) and Intel Macs running **macOS 11 Big Sur or newer**.

</div>

---

## 1. Install the Xcode Command Line Tools

These provide the C linker and macOS SDK that Rust needs. Open **Terminal** and run:

```sh
xcode-select --install
```

Click **Install** in the dialog. If it says the tools are already installed, you can skip this step.

## 2. Install Rust

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Choose the default installation (press <kbd>Enter</kbd>), then load Rust into your current shell:

```sh
source "$HOME/.cargo/env"
rustc --version   # should print rustc 1.7x or newer
```

## 3. Download DAEMON

```sh
git clone https://github.com/fcopensource/daemon.git
cd daemon
```

## 4. Run it

```sh
cargo run --release
```

The first build downloads and compiles the dependencies, which takes a few minutes. Later runs
start in seconds. DAEMON opens maximized, plays its boot sequence and drops you into your
normal shell (`zsh` on modern macOS).

Useful options:

```sh
cargo run --release -- --fullscreen        # start in fullscreen
cargo run --release -- --theme neon        # pick a theme
cargo run --release -- --mute              # start with sound off
DAEMON_SHELL=/bin/bash cargo run --release # use a different shell
```

---

## Make a real `DAEMON.app` (optional)

To get a double-clickable app with the **D** icon in your Dock and Launchpad:

```sh
cargo install cargo-bundle
cargo bundle --release
```

The app is written to `target/release/bundle/osx/DAEMON.app`. Move it into Applications:

```sh
cp -R target/release/bundle/osx/DAEMON.app /Applications/
```

Any files in the project's `sounds/` folder are copied into the app automatically (see
[Custom sounds](#custom-sounds)).

### Sharing the app with someone else

Apps you build yourself open normally. If you send `DAEMON.app` to another Mac, macOS
Gatekeeper may say it *"cannot be opened because the developer cannot be verified"*, because the
app isn't signed with an Apple Developer ID. The recipient can either:

- right-click the app → **Open** → **Open**, or
- clear the quarantine flag once:
  ```sh
  xattr -dr com.apple.quarantine /Applications/DAEMON.app
  ```

### Universal binary (Apple Silicon + Intel)

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin
lipo -create -output daemon-universal \
  target/aarch64-apple-darwin/release/daemon \
  target/x86_64-apple-darwin/release/daemon
```

---

## Keyboard on a Mac

| Keys | What happens |
|------|--------------|
| <kbd>Ctrl</kbd> + <kbd>C</kbd> | interrupt the running command (same as in Terminal.app) |
| <kbd>Cmd</kbd> + <kbd>V</kbd> | paste |
| <kbd>fn</kbd> + <kbd>F10</kbd> | sound on / off |
| <kbd>fn</kbd> + <kbd>F11</kbd> | fullscreen on / off (the green window button works too) |
| Trackpad scroll over the terminal | scroll back through output |

Most MacBook keyboards need <kbd>fn</kbd> for F-keys unless you turned on *"Use F1, F2, etc.
keys as standard function keys"* in **System Settings → Keyboard**.

---

## Custom sounds

Put `.wav`, `.ogg`, `.mp3` or `.flac` files in the `sounds/` folder to replace the built-in
sounds, for example `sounds/key.wav` for keystrokes or `sounds/ambient.mp3` for a looping
background track. See [`sounds/README.md`](sounds/README.md) for every file name.

---

## Troubleshooting

<details>
<summary><b><code>error: linker `cc` not found</code></b></summary>

The Xcode Command Line Tools are missing. Run `xcode-select --install`, then build again.
</details>

<details>
<summary><b><code>command not found: cargo</code></b></summary>

Rust isn't loaded in this terminal yet. Run `source "$HOME/.cargo/env"`, or open a new
Terminal window.
</details>

<details>
<summary><b>macOS asks for access to Desktop / Documents / Downloads</b></summary>

The file browser lists the folders in your home directory, and macOS asks for permission the
first time an app reads protected folders. Click **Allow** to see their contents, or
**Don't Allow** to keep them hidden. DAEMON still works either way.
</details>

<details>
<summary><b>No sound</b></summary>

- Check the top bar: **SOUND OFF** means it's muted; press <kbd>fn</kbd> + <kbd>F10</kbd>.
- **NO AUDIO** means no output device could be opened. Check **System Settings → Sound →
  Output** and make sure the volume is up.
</details>

<details>
<summary><b>The text looks too small or too large</b></summary>

Use <kbd>Cmd</kbd> + <kbd>+</kbd> / <kbd>Cmd</kbd> + <kbd>-</kbd> to zoom the whole interface,
and <kbd>Cmd</kbd> + <kbd>0</kbd> to reset.
</details>

<details>
<summary><b>Updating to the latest version</b></summary>

```sh
cd daemon
git pull
cargo run --release
```
</details>
