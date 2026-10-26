# Ferrum

A native, cross-platform SSH client written in Rust. It runs natively on macOS, Windows and Linux from a single codebase (egui + russh + alacritty_terminal).

## Features

- **Connections:** SSH, Mosh, Telnet, serial and a local shell, each in a tab. Panes split side by side, stacked or in a grid, with broadcast input to all panes and auto-reconnect.
- **Hosts:** groups with inherited settings, tags, pinning, search, and quick connect (`user@host:port`). Jump-host chains and SOCKS5 / HTTP / ProxyCommand proxies are supported. Each host can set a connect timeout and strict host-key checking.
- **SFTP:** a dual-pane browser for local ↔ remote and remote ↔ remote. It has a transfer queue with progress and cancel, and you can rename, change permissions, delete, create files and folders, or edit a file in your own editor, chosen per file type (changes upload on save).
- **Port forwarding:** local, remote and dynamic (SOCKS). Rules can start automatically when you connect to a host.
- **Keychain:** generate ED25519, ECDSA or RSA keys, import existing ones, and install a public key on a server. Identities let many hosts share one login, and the SSH agent works too.
- **Snippets:** reusable commands with `{{variables}}`, filed in packages (folders), each with an optional global hotkey. Run them in one terminal or several at once.
- **Workspaces:** save the open tabs, splits and hosts under a name and reopen them together. Tabs can also be duplicated.
- **Known hosts:** the app warns you when a server's key changes, and can import `~/.ssh/known_hosts`.
- **History and logs:** a searchable command history, a connection log, and optional session recordings.
- **Import:** OpenSSH config, PuTTY, MobaXterm, SecureCRT and CSV, plus cloud hosts from DigitalOcean, AWS EC2 and Azure.
- **Vault:** stored on disk, optionally encrypted with a master password (Argon2id + ChaCha20-Poly1305). It auto-locks when idle and supports encrypted export/import and folder sync.
- **Appearance:** dark, midnight and light themes, accent colours, 18 terminal colour schemes, custom fonts and keyword highlighting.
- **Command palette** (⌘K / Ctrl+K) and an optional AI command assistant.

## Build

```sh
cargo run --release
```

On Linux, install the GUI headers first:

```sh
sudo apt install libxkbcommon-dev libgl1-mesa-dev libwayland-dev libx11-dev libgtk-3-dev
```

## Tests

```sh
cargo test                       # unit tests
# End-to-end against a real SSH server (exec, SFTP round trip, local tunnel):
FERRUM_IT_SSH="user@127.0.0.1:2222" FERRUM_IT_KEY=~/.ssh/test_key \
  cargo test it_ -- --ignored --test-threads=1
```

CI (`.github/workflows/ci.yml`) runs both suites, starting a throwaway `sshd` for the integration tests, and builds release binaries on all three OSes.

## Visual QA

Debug builds can screenshot their own screens without screen-recording permission:

```sh
FERRUM_QA=home,sftp,settings,local,palette cargo run
# PNGs land in $TMPDIR/ferrum-qa (override with FERRUM_QA_DIR)
```

This mode adds demo data and never saves the vault. The steps `ssh`, `accept`, `type` and `sftpremote` use the live server from `FERRUM_IT_SSH` / `FERRUM_IT_KEY`.

## Credits

Icons are [Phosphor](https://phosphoricons.com) (MIT, `assets/fonts/PHOSPHOR-LICENSE.txt`). The UI palette is checked in tests for WCAG AA contrast in all three themes.

## Menus and updates

On macOS, File, Edit, View and Help live in the system menu bar. Windows and Linux show the same menus inside the window. Use Edit for Cut, Copy, Paste and Select all. Password inputs include a show/hide button.

Help → Ferrum help opens this page. Help → About Ferrum shows the installed version. Help → Check for updates checks published GitHub release tags. Preview releases are optional. For a private repository, Ferrum uses an existing GitHub CLI login, `GH_TOKEN` / `GITHUB_TOKEN`, or a token entered in the update dialog. Tokens entered there stay in memory until the app closes.

The updater selects a package for the current OS and CPU, checks its size and GitHub SHA-256 digest, then stages it before offering **Install and restart**. macOS additionally checks the app signature, bundle identifier and version. Restart saves the vault and closes terminal sessions. The old build is kept beside the installation as a hidden recovery copy. Installation errors leave a log in the `ferrum-update-*` temporary directory. AppImage and read-only installations can use the GitHub release page to install manually.

To publish an update, bump `package.version` in `Cargo.toml`, refresh `Cargo.lock`, and push the matching `v<version>` tag. CircleCI builds every platform, creates checksums, and publishes a GitHub Release after all builds succeed. The tag must match the package version.

## CircleCI

`.circleci/config.yml` runs format checks and unit tests, then builds packages for Linux x86_64, Windows x86_64, macOS Apple Silicon and macOS Intel. Branch builds store installers as CircleCI artifacts. Tags starting with `v` also run a publishing job after Linux, Windows, Apple Silicon and Intel Mac builds succeed.

Connect `fikrifirmanf/ferrum-ssh` in CircleCI and select the existing `.circleci/config.yml`. The macOS and Windows jobs need access to those CircleCI executors.

For automatic releases, add `GH_TOKEN` under **Project Settings → Environment Variables**. Use a fine-grained GitHub personal access token restricted to `fikrifirmanf/ferrum-ssh`, with **Contents: Read and write** permission. Give it an expiration date and rotate it before it expires. Keep it out of source files and build logs. GitHub Actions' release workflow is now a manual packaging fallback and does not publish releases.

After updating the version in `Cargo.toml` and refreshing `Cargo.lock`, commit and push `main`, then create and push the matching tag. For version `0.1.1`:

```sh
git tag -a v0.1.1 -m "Ferrum v0.1.1"
git push origin v0.1.1
```

The publisher checks that the GitHub tag points to the exact commit CircleCI built. It requires every installer, verifies the uploaded digests, and publishes only after all files are present. Failed uploads leave a draft for a retry. Published releases are never overwritten; an identical retry succeeds without changes. Tags such as `v0.2.0-beta.1` are published as preview releases.

The current branch pipeline cannot acquire a new publishing job midway through its run. Automatic publishing starts with a tag pipeline using this updated configuration.

To test the release helper locally, run `python3 -m unittest discover -s packaging -p 'test_*.py' -v`.

For interactive visual checks with disposable demo data, run `FERRUM_QA=interactive cargo run`. This mode keeps the window open and does not save the vault.
