# Norx repository manifest

This is the canonical repository/source-of-truth inventory for the shared
roadmap, including Roadmap 0.2 item S0-062. It is an inventory, not a release
claim: a project is not considered present because a build script, generated
ELF, package recipe, or smoke harness mentions its name.

Legacy inventory observed 2026-08-28 from workspace root
`/Users/alexandrbondarev/Documents/GitHub/norx`. Paths below are relative to
that root. `HEAD` is the checked-out local commit. A `dirty` marker means the
worktree had user/Codex changes at inventory time; those changes were not
inspected or rewritten as part of this manifest.

## ARM64 priority update (2026-10-10)

Native release architecture is aarch64 only. Existing x86 code, fixtures and
commands in the legacy inventory are archival diagnostics, unsupported as a
native product. x86/x86_64 in WLI means translated guest ISA. WLI is the first
priority utility port; Linuxulator, FEX/Wine/Proton remain prerequisites or
upstream candidates and cannot be inferred from launcher/classifier code.

## Field and status rules

- **Canonical remote/commit** is the Norx repository remote and observed
  checkout. For a Norx-owned repository, this is the source revision; it is
  not an external upstream claim.
- **Upstream remote/commit** is the external baseline recorded by the fork's
  `UPSTREAM.md`/recipe. `not recorded` is intentional and means the source
  coordinate is not yet pinned.
- **Owner** is only asserted when repository metadata supports it. The
  `NorxTeam` namespace is recorded as organization evidence, not as a named
  maintainer or CODEOWNERS declaration.
- **Source-of-truth status** uses these values:
  `canonical-source` (Norx-owned source), `forked-adapter` (Norx adapter in a
  maintained upstream fork), `workspace-source` (source owned here and
  packaged elsewhere), `fixture-only` (generated/test slot with no source),
  `planned-output` (future output of another repository), `missing` (no source
  or fixture), and `planned-external` (upstream candidate not yet selected or
  pinned).
- `none`/`—` means that no recipe, owner, coordinate, or test entrypoint was
  found in the workspace. It is an explicit gap, not an inferred value.

The roadmap's kernel/framework tracks (Sections 0--4) are not separate
repositories: their source of truth is `norx-kernel`, as shown in the first
row. The tables below cover every named standalone project and package unit in
Sections 5.1--5.13; related package outputs are grouped only where the
roadmap explicitly assigns them to one future source repository.

## Repositories present in the workspace

| Roadmap component | Actual source path | Canonical remote @ observed commit | Upstream remote @ recorded baseline | Owner | Package recipe / metadata | Test or build entrypoint | Source-of-truth status |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Kernel and Sections 0--4 | `norx-kernel/` | `origin https://github.com/NorxTeam/norx-kernel.git` @ `062a1e92fabba4ce8c7afbbcd954f7b68eac870d` (dirty) | none; Norx-owned | NorxTeam namespace; no named owner file found | `norx-kernel/Cargo.toml` (no Gamma recipe) | `cargo fmt --check`; target `cargo build`/`cargo clippy`; `MODE=build ./scripts/run.sh x86_64|aarch64`; feature smokes under `norx-kernel/scripts/` | `canonical-source` |
| Public ABI, runtime, Rust API, and source fixtures | `userspace/` (`include/`, `runtime/`, `rust/`, `examples/`) | `origin https://github.com/NorxTeam/userspace.git` @ `d5c942a9f253e8dd51e0b75d54b1315ea63064f2` (dirty) | none; Norx-owned | NorxTeam namespace; `README.md` defines ownership, no named owner file found | `userspace/runtime/gamma.toml`; `userspace/rust/gamma.toml` | `python3 toolchain/scripts/build-runtime.py`; `python3 toolchain/scripts/build-rust-userspace.py`; `cargo test --manifest-path userspace/rust/Cargo.toml` | `workspace-source` |
| C/C++/Rust cross-toolchain and SDK | `toolchain/` | `origin https://github.com/NorxTeam/toolchain.git` @ `4b63e5518ff03ddbcbcb06425accc363a23c625d` | none; Norx-owned | NorxTeam namespace; no named owner file found | `toolchain/gamma.toml`; `toolchain/abi.toml`; `toolchain/sdk.toml`; `toolchain/versions.toml` | `python3 toolchain/scripts/build.py`; `python3 toolchain/scripts/publish-sdk.py`; consumed by the runtime/fixture build scripts | `canonical-source` |
| `quickinit` service supervisor | `quickinit/` (`src/`) | `origin https://github.com/NorxTeam/quickinit.git` @ `af104dac0b385e7878924f8b490ce68aa5db0983` | none; Norx-owned | NorxTeam namespace; no named owner file found | `quickinit/gamma.toml`; bootstrap recipe `quickinit/bootstrap/gamma.toml` | `(cd quickinit && cargo test --locked)`; `python3 toolchain/scripts/build-quickinit.py` | `canonical-source` |
| `coreutils` Norx adapter | `coreutils/norx/` inside `coreutils/` | `origin https://github.com/NorxTeam/coreutils.git` @ `adcd299afc667eff2592a1a5a285fd5866730641` (dirty) | `upstream https://github.com/coreutils/coreutils.git` @ `f5e1fd312683e039985c733037f81e6c1378ac2a` | NorxTeam namespace; `UPSTREAM.md` is provenance, no named owner file found | `coreutils/gamma.toml`; `coreutils/norx/Cargo.toml` | `cargo test --manifest-path coreutils/norx/Cargo.toml`; `python3 coreutils/norx/scripts/build.py`; `python3 norx-kernel/scripts/coreutils-smoke.py x86_64|aarch64` | `forked-adapter` |
| `getty` Norx adapter | `getty/norx/` inside `getty/` | `origin https://github.com/NorxTeam/getty.git` @ `1e57ed2c5805750e1aea1206cc2c58bd17546b38` (dirty) | `upstream https://github.com/util-linux/util-linux.git` @ `53e442154c97b872b529a9f61e335d150ad0f742` | NorxTeam namespace; `AGENTS.md` has project guidance but no Norx owner declaration | `getty/gamma.toml`; `getty/norx/Cargo.toml` | `cargo test --manifest-path getty/norx/Cargo.toml --offline`; `python3 getty/norx/scripts/build.py`; `python3 norx-kernel/scripts/getty-smoke.py x86_64|aarch64` | `forked-adapter` |
| `login` Norx adapter | `login/norx/` inside `login/` | `origin https://github.com/NorxTeam/login.git` @ `7065beb0f9d38a93547950afbcb85f1db14de2d8` (dirty) | `upstream https://github.com/shadow-maint/shadow.git` @ `e4bd855661afe7c83ad2745d086a538398205225` | NorxTeam namespace; no named owner file found | `login/gamma.toml`; `login/norx/Cargo.toml` | `cargo test --manifest-path login/norx/Cargo.toml --offline`; `python3 login/norx/scripts/build.py`; `python3 norx-kernel/scripts/login-smoke.py x86_64|aarch64` | `forked-adapter` |
| `passwd` Norx adapter | `passwd/norx/` inside `passwd/` | `origin https://github.com/NorxTeam/passwd.git` @ `3a89e4302a00b54d0f5c58cbd6aceb0f7b42634c` (dirty) | `upstream https://github.com/shadow-maint/shadow.git` @ `e4bd855661afe7c83ad2745d086a538398205225` | NorxTeam namespace; no named owner file found | `passwd/gamma.toml`; `passwd/norx/Cargo.toml` | `cargo test --manifest-path passwd/norx/Cargo.toml --offline`; `python3 passwd/norx/scripts/build.py`; `python3 norx-kernel/scripts/passwd-smoke.py x86_64|aarch64` | `forked-adapter` |
| `userdb` Norx adapter | `userdb/norx/` inside `userdb/` | `origin https://github.com/NorxTeam/userdb.git` @ `32bf65561125f53e6b746f48cd6cbd4771e4660f` (dirty) | `upstream https://github.com/shadow-maint/shadow.git` @ `e4bd855661afe7c83ad2745d086a538398205225` | NorxTeam namespace; no named owner file found | `userdb/gamma.toml`; `userdb/norx/Cargo.toml` | `cargo test --manifest-path userdb/norx/Cargo.toml --offline`; `python3 userdb/norx/scripts/build.py`; `python3 norx-kernel/scripts/userdb-smoke.py x86_64|aarch64` | `forked-adapter` |
| `curl` fork | `curl/` | `origin https://github.com/NorxTeam/curl.git` @ `64070f0ad7cd16710fab96211608eae82ecc4dd8` (dirty from workspace noise) | `upstream https://github.com/curl/curl.git` @ `1cf6be5a0b4a1db8c501dcda8ac98e09970fd372` | NorxTeam namespace; no effective CODEOWNERS entries found | `curl/gamma.toml`; `curl/gamma.lock` (provenance-only, no target artifact) | `python3 curl/scripts/sync_upstream.py --check`; upstream host checks are part of that script; target/QEMU entrypoint is intentionally absent | `forked-adapter` (provenance-only; no Norx source patch or target port) |

The `coreutils/gnulib/` checkout is a Git submodule dependency, not a
roadmap-owned Norx repository. Its checked-out commit was
`6a1ec8e86781db40430adac7c9c7a2529cbac09b` and it is not a source of truth for
the Norx adapter.

## Roadmap projects without a source repository here

For this table, `—` in the owner and recipe columns is a recorded absence.
The planned `BoaKernel/<name>/` location is not treated as an existing path.

| Roadmap component | Actual source path | Canonical remote @ commit | Upstream remote @ commit | Owner | Package recipe | Test entrypoint | Source-of-truth status |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `norxshell` (`nsh`) | no source repository; expected generated slot `test-rootfs/tests/nsh/<triple>/nsh.elf` (root `test-rootfs/` is absent) | none | not recorded | — | none | `norx-kernel/scripts/nsh-smoke.py` / `nsh-smoke.sh` | `fixture-only`: smoke harness and kernel `build.rs` slot; no shell source, owner, or recipe |
| `userctl` | no source repository; expected generated slot `test-rootfs/tests/userctl/<triple>/userctl-smoke.elf` | none | not recorded | — | none | `norx-kernel/scripts/userctl-smoke.py`; `scripts/build-userctl-image.ps1` | `fixture-only`: kernel fixture slot only; not a shipped `userctl` source project |
| `sudo` | no source repository; expected generated slot `test-rootfs/tests/sudo/<triple>/sudo-smoke.elf` | none | not recorded | — | none | no dedicated source or smoke entrypoint; only kernel fixture wiring | `fixture-only`: kernel fixture slot only |
| `su` | no separate source repository; host upstream copies remain `login/src/su.c` and `passwd/src/su.c` | none | not recorded for a Norx `su` project | — | none | no Norx target entrypoint | `missing`: upstream source files are not the planned Norx `su` project |
| `session-utils` | absent | none | not recorded | — | none | none | `missing` |
| `syslogd` (and `dmesg`/`logctl`) | absent | none | not recorded | — | none | none | `missing` |
| `devd` | absent | none | not recorded | — | none | none | `missing` |
| `mountd` (and `mount`/`umount`) | absent | none | not recorded | — | none | none | `missing` |
| `timed` | absent | none | not recorded | — | none | none | `missing` |
| `netd` | absent | none | not recorded | — | none | none | `missing` |
| `netutils` | absent | none | not recorded | — | none | none | `missing` |
| `procutils` | absent | none | not recorded | — | none | none | `missing` |
| `storage-utils` | absent | none | not recorded | — | none | none | `missing` |
| `recoveryctl` | absent | none | not recorded | — | none | none | `missing` |
| `powerctl` | absent | none | not recorded | — | none | none | `missing` |
| `git` | absent | none | not recorded | — | none | none | `planned-external`: upstream baseline and Norx fork are not selected |
| `micro` | absent | none | not recorded | — | none | none | `planned-external`: upstream baseline and Go/native strategy are not selected |
| `gamma` | absent; `toolchain/gamma.toml` is only a staging recipe | none | not recorded | — | `toolchain/gamma.toml` is not a Gamma source recipe | none | `missing`: package-manager source repository does not exist |
| `nde` | absent | none | not recorded | — | none | none | `missing`: design/package-output specification only |
| NDE outputs: `nwayland`, `ndesession`, `nwidgets`, `npanel`, `nlauncher`, `nsettings`, `ncontrol`, `ngreeter`, `nthemes`, `nicons`, `kitty` | no separate path; intended to be independently packaged outputs from future `nde/` | none | not recorded | future NDE owner unassigned | none | none | `planned-output`: no NDE source repository exists |
| Qt foundation: `qtbase`, Qt GUI/Quick, Qt Wayland and platform backend | absent | none | not recorded | — | none | none | `planned-external`: packages are specified but no source coordinates are pinned |
| Gecko browser / Floorp | absent | none | `https://github.com/Floorp-Projects/Floorp` @ not selected | — | none | none | `planned-external`: roadmap names a source candidate but no Norx fork or commit exists |
| `ndrm` GPU/display ABI | not defined as a separate source; current low-level code is in `norx-kernel/src/drivers/display.rs` and related kernel files | none | none | norx-kernel maintainers to assign | none | kernel display smoke only; no `ndrm` ABI test | `missing`: planned ABI, not a repository |
| Mesa | absent | none | not recorded | — | none | none | `planned-external` |
| Khronos Vulkan Loader | absent | none | not recorded | — | none | none | `planned-external` |
| SPIR-V tools and shader compilers | absent | none | not recorded | — | none | none | `planned-external` |
| `wifid` | absent | none | not recorded | — | none | none | `missing` |
| `dnsd` / resolver | absent | none | not recorded | — | none | none | `missing` |
| PipeWire and client compatibility layers | absent | none | not recorded | — | none | none | `planned-external`: upstream port not selected |
| WirePlumber | absent | none | not recorded | — | none | none | `planned-external` |
| `audio-utils` | absent | none | not recorded | — | none | none | `missing` |

## Roadmap application and upstream-port portfolio

Every row below is a separate planned package/source unit unless the roadmap
explicitly says that several outputs share one future repository. No row has a
local path, owner, recipe, pinned commit, or test entrypoint today.

| Roadmap component | Actual source path | Upstream remote @ commit | Owner | Package recipe | Test entrypoint | Source-of-truth status |
| --- | --- | --- | --- | --- | --- | --- |
| `libarchive` (outputs `bsdtar`, `bsdcat`, `bsdcpio`, `bsdunzip`) | absent | not recorded | — | none | none | `planned-external` |
| `zstd` | absent | not recorded | — | none | none | `planned-external` |
| `gzip` | absent | not recorded | — | none | none | `planned-external` |
| `xz` | absent | not recorded | — | none | none | `planned-external` |
| `bzip2` | absent | not recorded | — | none | none | `planned-external` |
| `less` | absent | not recorded | — | none | none | `planned-external` |
| `findutils` (outputs `find`, `xargs`, `locate`) | absent | not recorded | — | none | none | `planned-external` |
| `jq` | absent | not recorded | — | none | none | `planned-external` |
| `ripgrep` | absent | not recorded | — | none | none | `planned-external` |
| `file` | absent | not recorded | — | none | none | `planned-external` |
| `tree` | absent | not recorded | — | none | none | `planned-external` |
| OpenSSL or selected TLS/crypto implementation | absent | not recorded | — | none | none | `planned-external` |
| `ca-certificates` | absent | not recorded | — | none | none | `planned-external` |
| `tzdata` | absent | not recorded | — | none | none | `planned-external` |
| OpenSSH (outputs `ssh`, `sshd`, `scp`, `sftp`) | absent | not recorded | — | none | none | `planned-external` |
| `rsync` | absent | not recorded | — | none | none | `planned-external` |
| `iwd` / `wpa_supplicant` (candidate `wifid` bases) | absent | not recorded | — | none | none | `planned-external` |
| `unbound` (candidate `dnsd` backend) | absent | not recorded | — | none | none | `planned-external` |
| `chrony` | absent | not recorded | — | none | none | `planned-external` |
| BlueZ | absent | not recorded | — | none | none | `planned-external` |
| CUPS | absent | not recorded | — | none | none | `planned-external` |
| WireGuard tools | absent | not recorded | — | none | none | `planned-external` |
| Avahi | absent | not recorded | — | none | none | `planned-external` |
| Python | absent | not recorded | — | none | none | `planned-external` |
| CMake | absent | not recorded | — | none | none | `planned-external` |
| Ninja | absent | not recorded | — | none | none | `planned-external` |
| Meson | absent | not recorded | — | none | none | `planned-external` |
| `pkg-config` | absent | not recorded | — | none | none | `planned-external` |
| GDB or LLDB | absent | not recorded | — | none | none | `planned-external` |
| `tmux` | absent | not recorded | — | none | none | `planned-external` |
| `make` | absent | not recorded | — | none | none | `planned-external` |
| Norx `strace`-like tracer | absent | none; Norx source not created | — | none | none | `missing` |
| Dolphin | absent | not recorded | — | none | none | `planned-external` |
| KDE Frameworks required by Dolphin | absent | not recorded | — | none | none | `planned-external` |
| FeatherPad | absent | not recorded | — | none | none | `planned-external` |
| mpv | absent | not recorded | — | none | none | `planned-external` |
| `btop` | absent | not recorded | — | none | none | `planned-external` |
| `grim` | absent | not recorded | — | none | none | `planned-external` |
| `slurp` | absent | not recorded | — | none | none | `planned-external` |
| `wl-clipboard` | absent | not recorded | — | none | none | `planned-external` |
| Ark | absent | not recorded | — | none | none | `planned-external` |
| `qpdfview` | absent | not recorded | — | none | none | `planned-external` (explicitly deferred) |
| LibreOffice | absent | not recorded | — | none | none | `planned-external` |
| Thunderbird or another Gecko mail client | absent | not recorded | — | none | none | `planned-external` (optional/deferred) |
| Blender | absent | not recorded | — | none | none | `planned-external` (optional/deferred) |
| OBS Studio | absent | not recorded | — | none | none | `planned-external` (optional/deferred) |
| `wli` | `wli/` | `https://github.com/NorxTeam/wli` @ `bb0c2652bd784582a21845ab9b73b03f6f3ab0bd` (pushed 2026-10-10; HOST/STAGED) | none; own classifier/manager, upstream backends not vendored | NorxTeam namespace | `wli/gamma.toml` staging recipe | `cargo test --manifest-path wli/Cargo.toml`; `python3 wli/scripts/build-norx.py` | `canonical-source`: ARM64 compile-only `diagnostics`/`inspect-fd` CLI; no guest execution |
| FEX translation backend inside `wli` | absent | https://github.com/FEX-Emu/FEX; selected first candidate, not pinned/ported | future WLI maintainer | none | none | `planned-external` |
| Box64 alternative inside `wli` | absent | https://github.com/ptitSeb/box64; evaluation only | future WLI maintainer | none | none | `planned-external` |
| Linuxulator-style ABI personality | not implemented | kernel prerequisite, not a launcher executable | Norx kernel maintainers | none | static ARM Linux hello/exit gate not passed | `missing` |
| Wine backend inside `wli` | absent | not recorded | future `wli` owner unassigned | none | none | `planned-output` |
| Proton backend inside `wli` | absent | not recorded | future `wli` owner unassigned | none | none | `planned-output` |
| DXVK backend inside `wli` | absent | not recorded | future `wli` owner unassigned | none | none | `planned-output` |
| VKD3D-Proton backend inside `wli` | absent | not recorded | future `wli` owner unassigned | none | none | `planned-output` |
| `runc` or `crun` | absent | not recorded | — | none | none | `planned-external` (later evaluation) |
| Podman or another container manager | absent | not recorded | — | none | none | `planned-external` (later evaluation) |
| Additional browsers, game runtimes, and launchers | absent | not selected | — | none | none | `planned-external` |

## Generated and non-source paths

These paths are deliberately called out so future inventory updates do not
mistake build output for a repository:

| Path or name | What exists | What it is not |
| --- | --- | --- |
| `test-rootfs/` | Expected staging destination used by build scripts; absent during this inventory | Not a checked-out repository or source of truth |
| `toolchain/build/` | Ignored generated sysroot and per-target artifacts when builds run; absent during this inventory | Not the SDK source; source is `userspace/` plus `toolchain/` metadata |
| `norx-kernel/target/` | Cargo build output, including historical `nsh.elf`/`userctl-smoke.elf` slots | Not `nsh` or `userctl` source |
| `norx-kernel/build.rs` | Kernel fixture lookup and bounded fallback-image generation | Not an implementation of `nsh`, `userctl`, or `sudo` |
| `toolchain/gamma.toml` | Staging metadata for the future package-manager hand-off | Not the `gamma` package-manager repository |
| `coreutils/gnulib/` | Upstream Git submodule dependency | Not a separate Norx roadmap component |

## Update rule

When a roadmap project changes path, remote, baseline, owner, recipe, or test
entrypoint, update this manifest in the same change as the source/recipe
change. Before a roadmap item is marked release-complete, its row must name a
real source path, a canonical revision, an owner, a package recipe, and a
real target test entrypoint; `fixture-only`, `missing`, `planned-output`, and
`planned-external` rows are not release evidence.
