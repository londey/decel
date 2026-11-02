# decel

A desktop spreadsheet where every cell can carry a Lua metatable that controls
how it computes, displays, edits, parses and validates its value. See
[DESIGN.md](DESIGN.md) for the specification and
[IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md) for the roadmap.

> **Status:** pre-alpha (Phase 0). Nothing useful to install yet.

## Install

```sh
cargo install decel
```

Lua and SQLite are vendored, so you only need a Rust toolchain (stable, 1.99 or
newer), a C compiler and GPUI's platform libraries.

### Build prerequisites

**Arch / CachyOS**

```sh
sudo pacman -S --needed base-devel clang cmake pkgconf \
    libxkbcommon libxkbcommon-x11 wayland libxcb libx11 \
    fontconfig freetype2 zstd vulkan-icd-loader alsa-lib
```

Plus the Vulkan driver for your GPU (`vulkan-radeon`, `vulkan-intel` or `nvidia-utils`).

**Debian / Ubuntu**

```sh
sudo apt install build-essential clang cmake pkg-config \
    libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
    libxcb1-dev libx11-xcb-dev libx11-dev \
    libfontconfig-dev libfreetype-dev libzstd-dev \
    libvulkan-dev mesa-vulkan-drivers libasound2-dev
```

**Fedora**

```sh
sudo dnf install gcc gcc-c++ clang cmake pkgconf-pkg-config \
    libxkbcommon-devel libxkbcommon-x11-devel wayland-devel \
    libxcb-devel libX11-devel libX11-xcb \
    fontconfig-devel freetype-devel libzstd-devel \
    vulkan-loader-devel mesa-vulkan-drivers alsa-lib-devel
```

**Windows**

Install [Rust](https://rustup.rs) with the MSVC toolchain and the
[Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
with the "Desktop development with C++" workload.

## Development

A devcontainer (`.devcontainer/`) provides the full Linux toolchain. Common tasks
are in the `justfile`:

```sh
just check   # fmt, clippy, tests, cargo-deny
just run     # run the app
```

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
