# Publishing Scope on Flathub: research and readiness assessment

Checked 11 October 2026 against official Flatpak and Flathub documentation and the current Scope repository. This is an internal research note, not application metadata, a manifest, or submission material. No Flatpak was built, no submission was opened, and no privileged package operation was tested.

## Decision before packaging

**Scope has a substantial eligibility blocker.** Flathub's current inclusion policy excludes host system utilities. Host-dependent and distribution-specific applications are also restricted, with some exceptions considered individually. Broad sandbox-incompatible applications require upstream support, which still does not guarantee acceptance. Scope manages host APT/Snap/Flatpak packages and is primarily targeted at Ubuntu. Our assessment is that these rules directly affect its present product design; only Flathub's reviewers can decide whether an exception is possible. Ask them about eligibility before investing in packaging. [Flathub requirements](https://docs.flathub.org/docs/for-app-authors/requirements#inclusion-policy)

There is a separate **AI-related eligibility risk**. Flathub prohibits AI-generated or AI-assisted manifests and AI-driven submission PRs, messages, and reviews. Known generated material in the app requires disclosure; research alone is exempt when its generated output is not incorporated into app/packaging material. [Generative AI policy](https://docs.flathub.org/docs/for-app-authors/requirements#generative-ai-policy)

More strictly, Flathub says sandbox-escape exceptions will not be granted when there are signs of LLM use in the software or exception PR. LLM assistance with exception PRs can result in permanent denial. A human-written manifest does not remove that separate condition. The author must establish Scope's actual development history and discuss its implications honestly; this research cannot determine how reviewers would assess that history. [Exception policies](https://docs.flathub.org/docs/for-app-authors/linter#exception-policies)

## The sandbox problem

Flatpak's normal execution environment exposes the packaged app and runtime, with limited host files, processes, devices, and services. `/usr` inside the sandbox is the runtime rather than the host operating system. Host operating-system files can be mapped under `/run/host`; some paths, including other apps' `~/.var/app` and Flatpak installation directories, need explicit access. A permission alone does not fix code that assumes native host paths. App-private XDG cache/config/data directories should remain private. [Sandbox permissions](https://docs.flatpak.org/en/latest/sandbox-permissions.html)

`flatpak-spawn --host` runs an unsandboxed command on the host and requires access to `org.freedesktop.Flatpak` on the session bus. It supports explicit environment control, working directories, and `--watch-bus`; ordinary subprocess execution remains inside the sandbox. [Flatpak command reference](https://docs.flatpak.org/en/latest/flatpak-command-reference.html#flatpak-spawn)

Flathub's linter documents exceptions for broad host/home access and Flatpak D-Bus access. Flatpak-name exceptions require a sufficient rationale and are limited to Flatpak clients/development applications. These permission exceptions are separate from admission of the application itself. Scope's Flatpak-client functionality does not automatically authorize its APT/Snap host-management functionality. [Linter permission exceptions](https://docs.flathub.org/docs/for-app-authors/linter#finish-args-flatpak-talk-name)

**Engineering assessment, not a tested implementation:** retaining all current features would require a host-aware command and filesystem layer, or a separately installed host service. A host service would itself introduce the host dependency/post-installation concerns above. Running the entire Scope UI on the host from a minimal Flatpak launcher would surrender the sandbox and is not a credible solution to the admission problem.

## Scope repository assessment

Scope is currently version `0.3.5`, licensed MIT, with a committed `Cargo.lock`. It has an existing desktop file and SVG assets, but no Flatpak manifest or AppStream MetaInfo file. Its release workflow produces `.deb` and AppImage artifacts; that workflow does not publish to Flathub. The current desktop launcher uses `Exec=scope-gpui`, `Icon=scope`, and `Categories=System;`. The GPUI window uses the short app ID `scope`.

The following is implementation analysis from the repository, not a Flathub compatibility test:

| Area | Required investigation/change if reviewers permit this product |
| --- | --- |
| Command execution | `src/domain/system/mod.rs` directly constructs subprocesses. Scans, probes, APT simulation, preview support, and apply must consistently target the host. Executable discovery currently consults sandbox paths and user-local binaries; it needs an explicit host context. |
| Elevation | Host APT/Snap/system Flatpak operations must invoke host `pkexec`, preserve the existing argv-only commands and authorization agent behavior, and handle cancelled authentication. Bundling `apt`, `snap`, or `pkexec` inside the sandbox would not itself operate on the host. |
| Process lifecycle | Host subprocess timeout, cancellation, streamed logs, exit status, and child cleanup need verification. In particular, cancelling a `flatpak-spawn` wrapper must not leave a privileged package transaction running unnoticed. Forward simulation's `LC_ALL=C` explicitly rather than assuming wrapper environment propagation. |
| APT inspection | dpkg-listed absolute paths followed by local filesystem measurements could accidentally measure runtime `/usr` files. Host file identities, conffiles, symlinks, and size measurements must all use the same host context. Never replace bounded attributable measurements with broad directory traversal. |
| Snap inspection | `/snap`, `/var/lib/snapd`, and `/run/snapd.socket`, plus the subprocess used to query snapd, require host-aware access. Keep current read-only, bounded behavior. |
| Flatpak inspection | Keep `install_scope` explicit in every host command. Host deployment paths returned by `flatpak info` need correct local mapping or host-side measurement; user and system installations must remain distinct. |
| Desktop discovery | Current XDG discovery reflects the sandbox environment. Separate host launcher search paths from Scope's own private state directories. |
| AppImage discovery | Check host `/opt`, user discovery directories, and exact portable sidecars through the host context. AppImage operations must remain protected as required by the existing product rules. |
| Icons | Resolve host icon paths, map them into readable sandbox paths, and preserve the `scope-icon://` producer/decoder contract. A valid host path is not necessarily a GPUI-readable sandbox path. |
| Self-update | Add explicit Flatpak installation detection. The Flathub installation should update through Flatpak; do not invoke the native `.deb`/AppImage installer or mistake runtime files for native installation markers. |
| App identity | Align the chosen Flatpak ID with desktop/MetaInfo/icon identity and the GPUI window app ID so launchers, Wayland identity, and window grouping agree. |

Scope's safety contract remains mandatory in any port: preview-only plan creation, single-use expiring plan IDs, apply revalidation, protected-package checks, unchanged update versions, and repeat matching APT simulations. Changes to command routing/probe/revalidation need targeted Rust tests, plus the repository's `cargo check`, `cargo test`, and `cargo clippy` checks. These are Scope requirements, additional to Flatpak testing.

## Identity and verification

The recommended candidate is **`io.github.khurrambhutto.scope`**, derived from the current repository owner/name. Confirm the repository URL and ownership before committing to it; preserve the repository's lowercase `scope` component. Alternatively use a reverse-DNS ID based on a domain you control. Choose once: changing an accepted ID entails resubmission. [Application ID rules](https://docs.flathub.org/docs/for-app-authors/requirements#application-id)

After acceptance, an app maintainer can sign into Flathub's Developer Portal and verify the app. `io.github.` identity can use the repository owner's GitHub login; organizational repositories require an organization admin. Domain IDs use an HTTPS well-known token file or DNS TXT record. Verification means the publisher is upstream/authorized; it is distinct from admission or a security guarantee. No new `com.github.` IDs are accepted. [Verification](https://docs.flathub.org/docs/for-app-authors/verification)

## Required upstream metadata

Prepare and install an ID-named desktop entry, icon, and `ID.metainfo.xml`. MetaInfo needs a desktop-application component, exact app ID, desktop launchable, name/summary/description, developer identity, project and metadata licenses, OARS rating, release history, and screenshots. MIT is the appropriate project-license value for the current code; choose a separate valid metadata license. Screenshots need directly reachable image URLs; git-hosted images should be pinned to a tag/commit. Validate using Flathub's AppStream linter; warnings are fatal too. [MetaInfo guidelines](https://docs.flathub.org/docs/for-app-authors/metainfo-guidelines)

These metadata files belong upstream, rather than copied into the submission. Redistribution and installed module-license requirements also apply. The root manifest must be named after the app ID and target the newest available supported Flathub runtime. [Required files and licenses](https://docs.flathub.org/docs/for-app-authors/requirements#required-files)

For better presentation, capture real Linux app windows with normal decorations, without promotional overlays or desktop wallpaper, and show populated useful views. For Scope, the list, expanded footprint, and uninstall preview are sensible subjects. SVG or square PNG at least 256 pixels is recommended for listing quality. Branding, concise captions, current screenshots, and informative release notes improve curation; the quality guidelines distinguish these promotion checks from basic admission. [Quality guidelines](https://docs.flathub.org/docs/for-app-authors/metainfo-guidelines/quality-guidelines)

## Rust/GPUI packaging plan, conditional on eligibility

The Freedesktop runtime is the natural first candidate for a native GPUI application without GTK/Qt requirements. It supplies the toolchain/graphics foundation, has annual major releases and two-year branch support. GNOME/KDE add their respective stacks. Do not choose a runtime version merely because an older tutorial uses it. [Flathub runtimes](https://docs.flathub.org/docs/for-app-authors/runtimes)

On 11 October 2026, `flatpak remote-ls flathub --runtime --columns=application,branch` confirmed `26.08` for `org.freedesktop.Platform`, `org.freedesktop.Sdk`, and `org.freedesktop.Sdk.Extension.rust-stable`. Start compatibility work with that branch and recheck availability before submission. The extension supplies the Rust toolchain at its documented SDK path; LLVM may also be needed by Scope's native dependencies. [Rust SDK extension](https://github.com/flathub/org.freedesktop.Sdk.Extension.rust-stable)

Cargo dependencies can be prepared from `Cargo.lock` with Flatpak's cargo source generator, or released as an upstream vendored source archive. Generated sources go into the packaging sources; use the generated Cargo configuration and offline Cargo build. Git dependencies require special attention to source replacement/configuration. Regenerate dependency sources whenever the lockfile changes. [Cargo generator](https://github.com/flatpak/flatpak-builder-tools/blob/master/cargo/README.md)

**Scope-specific build investigation:** compare the chosen SDK/runtime contents with the release workflow's native dependencies, especially fontconfig, FreeType, X11/XCB, Wayland, xkbcommon-x11, Vulkan/graphics, and any compiler/bindgen needs. Bundle missing libraries from source as modules; keep bundled Work Sans and its OFL license. Prove GPUI rendering under both Wayland and X11. Linux display guidance uses Wayland with fallback X11 where supported, IPC for X11 shared memory, and DRI for graphics. Avoid irrelevant audio, full bus, device-all, or full filesystem permissions. [Flatpak display permissions](https://docs.flatpak.org/en/latest/sandbox-permissions.html#standard-permissions)

The deliverable would be a human-authored root manifest plus deterministic source URLs/checksums or pinned commits and the Cargo dependency sources. Existing `.deb`/AppImage binaries are not the source-build input. This note intentionally supplies no manifest or submission text.

## Local validation commands

The local machine has Flatpak and a configured Flathub remote; `org.flatpak.Builder` is not installed and a standalone `flatpak-builder` executable was not found. Nothing was installed during research.

After eligibility and implementation are settled, the documented build/test sequence is:

```bash
flatpak remote-add --if-not-exists --user flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak install --user flathub org.flatpak.Builder
flatpak run --command=flathub-build org.flatpak.Builder --install <manifest>
flatpak run <app-id>
flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest <manifest>
flatpak run --command=flatpak-builder-lint org.flatpak.Builder repo repo
flatpak run --command=flatpak-builder-lint org.flatpak.Builder appstream <app-id>.metainfo.xml
```

The submission documentation recommends this Builder-based workflow. [Build and lint](https://docs.flathub.org/docs/for-app-authors/submission#before-submission)

**Test plan proposal:** first prove correct host inventory without changing packages, including absent APT/Snap on non-Ubuntu systems, scope-separated Flatpak installs, paths/icons, refresh, cancellation, and read-only footprint accuracy. Then test authorized preview/apply on disposable VMs with removable sample packages, protected/indirect removals, version changes, failed simulations, auth cancellation, and timeouts. Test each supported architecture and graphics/session combination. This is proposed engineering work, not completed validation.

## Submission and maintenance lifecycle

A human forks `flathub/flathub` including its `new-pr` branch, creates a submission branch from `new-pr`, adds packaging files, and opens the GitHub web PR against **`new-pr`, not `master`**. Follow the template and respond to volunteer reviewers. After issues are resolved, `bot, build` requests a test build. Acceptance creates a separate app repository and sends a collaborator invitation; enable GitHub 2FA and accept within one week. There is no guaranteed review deadline. Published builds usually appear within hours, subject to queue/load. [Submission process](https://docs.flathub.org/docs/for-app-authors/submission#submission-pr)

New apps enter stable first; beta is available only after acceptance. Nightlies/daily development snapshots are unsuitable. [Stable-release policy](https://docs.flathub.org/docs/for-app-authors/requirements#stable-releases)

Subsequent versions use PRs in the app's own Flathub repository. Test the PR build before merging; merge triggers an official build and publication. Git `master` maps to Flatpak stable, `beta` to beta. Permission/critical-metadata changes can be held for moderation. Architecture defaults are x86_64 and aarch64; `flathub.json` can restrict them. External Data Checker can propose source updates, but Cargo-source regeneration may need additional deterministic tooling. Maintainers remain responsible for runtime upgrades, issues, and functional builds. [Maintenance](https://docs.flathub.org/docs/for-app-authors/maintenance)

## Unresolved decisions and recommended order

1. The author should independently contact reviewers about whether a unified privileged host package manager is eligible, including the need for host execution and the applicable AI/exception history. This research does not constitute approval.
2. If reviewers decline the present product, keep `.deb`/AppImage distribution or assess a materially different Flatpak-focused product. Removing APT/Snap would change Scope's purpose and does not itself guarantee acceptance.
3. If an acceptable design is established, agree on its exact host-access mechanism and permission rationale before implementing it. Do not rely on users manually overriding the sandbox for core functionality.
4. Confirm permanent app identity, supported distributions/architectures, maintenance ownership, and current runtime/toolchain compatibility.
5. Implement host-aware behavior and metadata, test the package, and have a human write/review all submission and exception material under the current policies.

Self-hosting Flatpak packages is technically separate from Flathub admission and would still require a functioning sandbox/host design. Flatpak documents both self-hosted repositories and single-file bundles; neither provides a Flathub listing. [Publishing repositories](https://docs.flatpak.org/en/latest/repositories.html), [Single-file bundles](https://docs.flatpak.org/en/latest/single-file-bundles.html)
