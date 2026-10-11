<p align="center">
  <img src="assets/scope-logo.svg" alt="Scope logo" width="96" />
</p>

<h1 align="center">Scope</h1>

<p align="center"><strong>See, update, and uninstall every app on your Linux system in one place.</strong></p>

<p align="center">
  <a href="https://khurrambhutto.github.io/scope/">Website</a> ·
  <a href="https://github.com/khurrambhutto/scope/releases">Releases</a> ·
  <a href="https://github.com/khurrambhutto/scope/issues">Issues</a>
</p>

Scope puts apps from APT, Snap, Flatpak, AppImage, uv, pipx, Cargo, and user-local desktop launchers into one searchable list. You can update or uninstall supported apps, and you see the exact commands before anything runs. Privileged steps go through the standard Polkit dialog, so Scope never sees your password.

<p align="center">
  <a href="docs/image.png"><img src="docs/image.png" alt="Scope showing installed apps, search, source filter, and Show all in the top bar" width="49%" /></a>
  <a href="docs/app-details.png"><img src="docs/app-details.png" alt="Scope with App Center expanded to show application files, older revisions, user data, system data, and estimated removal size" width="49%" /></a>
</p>

Scope is written in Rust with Zed's GPUI. It has no webview. Architecture and module rules live in [AGENTS.md](AGENTS.md).

## Install

The current release is [v0.3.5](https://github.com/khurrambhutto/scope/releases/tag/v0.3.5). Scope targets Ubuntu on x86-64 first.

Download [scope_0.3.5_amd64.deb](https://github.com/khurrambhutto/scope/releases/download/v0.3.5/scope_0.3.5_amd64.deb), then run this from the download folder:

```bash
sudo apt install ./scope_0.3.5_amd64.deb
```

For a portable build, download [Scope-0.3.5-x86_64.AppImage](https://github.com/khurrambhutto/scope/releases/download/v0.3.5/Scope-0.3.5-x86_64.AppImage), make it executable, and launch it:

```bash
chmod +x Scope-0.3.5-x86_64.AppImage
./Scope-0.3.5-x86_64.AppImage
```

Scope checks GitHub Releases on startup. Official `.deb` builds from v0.3.4 can install later updates after verifying a signed checksum manifest. If you upgrade from an older version that has no embedded verification key, install v0.3.5 by hand. Other install types open the Releases page when no verified package is available.

### Build from source

Install Rust stable and the Linux build dependencies, then clone and run the app:

```bash
sudo apt install build-essential pkg-config libfontconfig1-dev libx11-dev libx11-xcb-dev \
  libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev wayland-protocols
git clone https://github.com/khurrambhutto/scope.git
cd scope
cargo run
```

## Using Scope

Use the **Apps / Updates** toggle at the top to switch between installed apps and available updates. Search by app or package name, filter by source, or refresh to rescan. Select a row to see its details and the actions it supports.

The default list shows GUI apps and command-line tools that you would expect to manage. AppImages and user-local desktop launchers appear for discovery only, and their update and uninstall actions are disabled.

Turn on **Show all** to the right of the search bar to see every row the scanners return, including libraries and system components. Show all does not bypass safety checks. Protected packages stay locked even when it is on.

Every update or uninstall follows the same steps:

1. Scope builds a plan that lists the exact commands.
2. You review the plan and confirm it.
3. Scope checks the package against the live system.
4. Scope runs the command and streams the log to the footer.

Only one package operation can run at a time.

## Supported sources

| Source | Scanned | Uninstall | Update |
| --- | --- | --- | --- |
| APT | Manual installs only | `apt remove` | `apt install` |
| Snap | Apps only, runtimes hidden | `snap remove` | `snap refresh` |
| Flatpak | User and system installs, scoped separately | `flatpak uninstall` | `flatpak update` |
| uv tools | Persistent user tool environments | `uv tool uninstall` | `uv tool upgrade` |
| pipx | User environments, including suffixed installs | `pipx uninstall` | `pipx upgrade` |
| Cargo | Tracked user binary crates | `cargo uninstall` | `cargo install` with retained source and build settings |
| AppImage | Common locations | Not yet supported | Not yet supported |
| Desktop launchers | Launchers no package manager owns | Disabled | Disabled |

Scope runs each package manager's own command. It never edits dpkg or apt databases directly.

uv, pipx, and Cargo tools use the same list, filters, footprint panel, confirmation modal, and operation footer as other sources. One row represents a tool environment or crate, even when it exposes several commands. Scope retains the manager executable, installation root, and receipt identity, then checks them again before applying a plan. User tool operations run without administrator authorization.

uv update checks resolve the original requirements, constraints, overrides, and index options without building packages. Upgrades respect the original constraints. pipx updates retain recorded pip options and upgrade the main package; injected packages are listed in removal previews and are removed with their environment. pipx can drop original version constraints during upgrade, while explicitly pinned packages remain pinned.

Cargo update checks cover crates.io releases and tracked Git branches. Rebuilds retain binary selection, features, profile, target, and version constraints. Cargo subcommands and Rust toolchain components are not installed-tool rows. Cargo builds may take longer than Python upgrades and stream their output while browsing remains available.

Management requires verified user-owned roots inside your home directory. Shared caches, interpreters, toolchains, personal settings, and app-created data are kept. Global pipx installs, untracked Cargo binaries, and incomplete ownership records are not managed. Git/local Python sources, local Cargo sources, and custom Cargo registries have no automatic release check in this implementation; details explain the limitation, and verified installations can still be uninstalled. Failed or timed-out update checks remain distinct from an up-to-date result.

The optional native-manager checks use disposable installations in private temporary roots:

```bash
cargo test tool_managers::live_tests::disposable -- --ignored
```

They require uv, pipx, Cargo, Git, and Python. pipx may download packaging libraries. They do not update or uninstall existing working tools.


## Status

What works today:

- One list across APT, Snap, Flatpak, AppImages, uv, pipx, Cargo, and unmanaged desktop launchers
- Display names, categories, and icons from desktop entries and the freedesktop icon theme
- Uninstall with a preview, Polkit authentication, and a deny-list for protected packages
- Snap removal details showing installed revisions, managed data sizes, and retained recovery snapshots
- Update with a preview for APT, Snap, and Flatpak
- A self-updater that verifies signed checksums for official `.deb` builds

What is not done yet:

- `.rpm` release packages (these installs fall back to a manual download)
- AppImage uninstall and update
- Snap target versions in the update preview
- Whole-system cleanup and disk usage views
- Fedora and Arch package-manager support

## Safety

- **Preview first.** Every destructive action builds a plan. Applying a plan works only once, and a plan expires after five minutes.
- **Revalidate before running.** Scope checks that the package is still present, that an update's version has not changed, and that the deny-list still allows the action. If it cannot verify the state, it stops.
- **Simulate APT first.** Each APT transaction runs in a simulation. Scope rejects any removal that would take a protected package with it, and it repeats the simulation right before applying.
- **Protect system packages.** The deny-list covers critical APT packages, kernel images, Snap runtimes, and AppImage paths outside the allow-list.
- **Use Trash.** Deleted files go to the Trash through `gio trash`, with a manual fallback to `~/.local/share/Trash`.

## Development

```bash
cargo run
cargo check
cargo test
cargo clippy
```

Run all four before you open a pull request. Changes to the backend's safety code, such as probing, revalidation, the deny-list, or the plan store, need targeted Rust tests in the same change.

Good first contributions:

- AppImage auto-update in `src/domain/operations/update.rs`
- Snap target versions in `src/domain/scanner/snap.rs`
- `.rpm` release artifacts in `.github/workflows/release.yml`
- New scanners that implement the `Scanner` trait in `src/domain/scanner/`

Keep feature logic in domain modules. `main.rs` and `ui/app_view.rs` only compose and wire. `backend.rs` stays a thin orchestration layer.

## Releases and signing

Releases publish an amd64 `.deb`, an x86-64 AppImage, `SHA256SUMS`, and `SHA256SUMS.minisig`. RPM, Snap, and Flatpak builds are not published.

Push a plain version tag such as `v0.3.5` to run the release workflow. It publishes a GitHub release titled `Scope v0.3.5` and refuses tags that do not match `Cargo.toml`. It hashes every package artifact and signs `SHA256SUMS` with minisign. The public key comes from the `SCOPE_UPDATE_PUBLIC_KEY` repository variable and is compiled into release builds. The private key lives only in the `SCOPE_UPDATE_SECRET_KEY` repository secret. Never commit it.

To rotate keys, generate a new pair, replace both GitHub values, and publish a release through the normal workflow. Existing builds trust the key they were built with, so users on those builds must update by hand across a rotation.

## Contributing

Pull requests are welcome. Please keep commits in conventional form (`feat:`, `fix:`, `docs:`, `chore:`, `style:`), and keep each change limited to what it needs.

## License

[MIT](LICENSE)
