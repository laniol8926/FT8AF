# Desktop build for 32-bit ARM (Raspberry Pi Zero 2W)

Confirmed working end to end on a Raspberry Pi Zero 2W (armv7l, Raspbian
Bookworm, `labwc`/Wayland desktop): the app launches, the waterfall renders,
and FT8 decodes appear with a QRP Labs QMX connected via Hamlib.

There is no native armhf toolchain on the build host (an aarch64 SBC) and
GitHub's hosted arm64 runners have no 32-bit ARM execution support at all, so
this is a pure cross-compile, done in a `debian:bookworm` Docker container.

## Why this needs more than `apt-get install`

Installing the full `libwebkit2gtk-4.1-dev`/`libgtk-3-dev` `:armhf`
dependency chain with `apt-get install` fails outright on Debian Bookworm,
for two independent reasons:

1. **`binutils` and `binutils:armhf` have a hard, mutual `Conflicts:`** in
   Debian's own package metadata. A cross-compiler (`gcc-arm-linux-gnueabihf`,
   which needs native `binutils`) can never be installed alongside
   `build-essential:armhf`/any native-armhf-toolchain package
   (`gobject-introspection:armhf`'s own chain needs the latter) — confirmed
   directly with `apt-get install -s binutils binutils:armhf`.
2. **`gobject-introspection:armhf` depends on `python3-mako:armhf`/
   `python3-markdown:armhf`**, which don't exist as architecture-qualified
   packages in Bookworm at all — a real packaging gap, not fixable by flag
   or package selection.

The fix: never request a native-armhf-toolchain package, and fetch the
GTK/WebKit runtime+dev libraries with `apt-get download <pkg>:armhf` +
`dpkg -x <file>.deb /sysroot` instead of `apt-get install` — this bypasses
dpkg's whole-system dependency resolver entirely (nothing is "installed",
just extracted as loose files), sidestepping both blockers.

## The three build-time fixes that actually matter

1. **`libc6`/`libc6-dev` are "Multi-Arch: same"** (unlike `binutils`), so
   installing `libc6:armhf`/`libc6-dev:armhf` for real (not just
   sysroot-extracted) is safe and necessary — the cross-linker needs their
   real absolute paths (`/lib/arm-linux-gnueabihf/`,
   `/usr/lib/arm-linux-gnueabihf/`), since glibc's paths are hardcoded into
   the cross-toolchain's default search path. Also needed:
   `libc6-dev-armhf-cross` + `linux-libc-dev-armhf-cross` for cross headers
   (fixes `openssl-sys` building from source).

2. **`-Wl,--allow-shlib-undefined` is the real unlock.** No hand-built
   sysroot can cheaply replicate WebKitGTK's full transitive dependency
   closure (gstreamer, ICU, libsecret, libgcrypt, X11 extensions, libxslt,
   libjpeg/png, libselinux, libsystemd, pcre2, libavif, libmanette, nghttp2,
   wayland, freetype, drm/gbm...) — there's always another layer. The fix is
   to stop trying: this linker flag defers those symbols to the *real*
   device's runtime, which will have a complete, consistent armhf userspace
   via its own normal `apt-get install` of the GTK/WebKit stack. Set via
   `CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_RUSTFLAGS`.
   - Caveat: this doesn't cover a *static* archive linked directly into the
     binary. One case needed it anyway: `libdbus-1.a` needs
     `sd_listen_fds`/`sd_is_socket` from libsystemd at link time — fixed
     with an explicit `-lsystemd` plus a manually-created
     `libsystemd.so -> libsystemd.so.0` symlink (only the runtime package
     had been extracted, not `-dev`, which normally provides that symlink).

3. **`--features tauri/custom-protocol` is required and easy to miss.** A
   successful cross-compile without it produces a binary that launches fine
   but shows "Could not connect to localhost: Connection refused" instead of
   the real UI. Tauri v2 decides dev-server vs. embedded-frontend purely by
   whether the `tauri` crate itself was built with this Cargo feature (see
   `tauri-build`'s `is_dev()`) — **not** by cargo's debug/release profile and
   **not** by any `TAURI_ENV_DEBUG` env var. This project's `Cargo.toml` has
   no feature passthrough for it, so enable it directly on the command line
   with cargo's `pkg/feature` syntax.

## Full build recipe

```bash
# One-time container setup (debian:bookworm):
#  - native build-essential, gcc-arm-linux-gnueabihf, pkg-config, Node.js, Rust
#    (rustup target add armv7-unknown-linux-gnueabihf)
#  - libc6-dev-armhf-cross, linux-libc-dev-armhf-cross
#  - libc6:armhf, libc6-dev:armhf (installed for real, confirmed conflict-free
#    with `apt-get install -s` first)
#  - a /sysroot populated via `apt-get download <pkg>:armhf` + `dpkg -x` for
#    libwebkit2gtk-4.1-dev, libgtk-3-dev, librsvg2-dev,
#    libayatana-appindicator3-dev, libudev-dev, libasound2-dev and their
#    transitive closure (architecture:all packages like x11proto-dev go in
#    WITHOUT the :armhf suffix)
#  - a manual `ln -sf libsystemd.so.0 /sysroot/usr/lib/arm-linux-gnueabihf/libsystemd.so`

npm --prefix desktop ci
npm --prefix desktop run build   # build.rs embeds desktop/dist

source "$HOME/.cargo/env"
export PKG_CONFIG_ALLOW_CROSS=1
export PKG_CONFIG_LIBDIR=/sysroot/usr/lib/arm-linux-gnueabihf/pkgconfig:/sysroot/usr/share/pkgconfig
export PKG_CONFIG_SYSROOT_DIR=/sysroot
export CC_armv7_unknown_linux_gnueabihf=arm-linux-gnueabihf-gcc
export CXX_armv7_unknown_linux_gnueabihf=arm-linux-gnueabihf-g++
export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_LINKER=arm-linux-gnueabihf-gcc
export CARGO_TARGET_ARMV7_UNKNOWN_LINUX_GNUEABIHF_RUSTFLAGS="-C link-arg=-Wl,--allow-shlib-undefined -C link-arg=-lsystemd"

cd desktop/src-tauri
cargo build --release --target armv7-unknown-linux-gnueabihf --features tauri/custom-protocol
```

The resulting binary is at
`desktop/src-tauri/target/armv7-unknown-linux-gnueabihf/release/ft8af` — a
dynamically-linked ARM EABI5 executable. Copy it to the Pi and run it; it
needs the device's own `libwebkit2gtk-4.1-0`, `libgtk-3-0`,
`libayatana-appindicator3-1`, `libasound2`, `libudev1` (etc.) already
installed, same as any other armhf desktop app — the build-time `/sysroot`
above is cross-compile scaffolding only and isn't shipped.

Full build takes roughly 35-40 minutes on a 4-core aarch64 SBC; incremental
rebuilds that only touch this crate are much faster (minutes, not tens of
minutes).

## Hamlib (QMX support)

The Pi's distro `libhamlib4` package (4.5.4 on Raspbian Bookworm) has **no
QRP Labs QMX support at all** — QMX support (a Kenwood-TS480-style CAT
backend, `rigs/kenwood/ts480.c`) only exists from Hamlib 4.7.1 onward.
`rig.rs`'s `load_hamlib()` already anticipates this: on Linux it tries the
absolute path `/usr/local/lib/libhamlib.so.4` *before* the bare
`libhamlib.so.4` soname, specifically so a from-source build doesn't
conflict with (or lose to) whatever the distro package provides.

Cross-compile Hamlib the same way, reusing the toolchain above:

```bash
curl -sL -o hamlib-4.7.1.tar.gz \
  https://github.com/Hamlib/Hamlib/releases/download/4.7.1/hamlib-4.7.1.tar.gz
tar xf hamlib-4.7.1.tar.gz && cd hamlib-4.7.1

export PKG_CONFIG_ALLOW_CROSS=1
export PKG_CONFIG_LIBDIR=/sysroot/usr/lib/arm-linux-gnueabihf/pkgconfig:/sysroot/usr/share/pkgconfig
export PKG_CONFIG_SYSROOT_DIR=/sysroot
./configure --host=arm-linux-gnueabihf --prefix=/usr/local --disable-static \
  CC=arm-linux-gnueabihf-gcc
make -j4
```

The C++ bindings (`c++/`) fail to link ("file in wrong format") because
`CXX` isn't cross-set — this doesn't matter, since the app only `dlopen`s
the plain C library via the `libloading` crate and never touches the C++
bindings. The core library (`src/.libs/libhamlib.so.4.0.7`) builds and links
fine before the C++ directory is even reached; grab that file directly.
`arm-linux-gnueabihf-strip` shrinks it from ~22MB to ~9.6MB.

Install it on the device at the exact path the app checks first:

```bash
sudo mv libhamlib.so.4.0.7 /usr/local/lib/libhamlib.so.4.0.7
sudo ln -sf libhamlib.so.4.0.7 /usr/local/lib/libhamlib.so.4
sudo ldconfig
```

No `LD_PRELOAD`, no touching the distro package, no PATH changes — the
absolute-path check in `load_hamlib()` does the rest. Restart the app so it
picks up the new library.

## Device desktop environment notes (Raspberry Pi OS, `labwc`)

Raspberry Pi OS's current desktop runs `labwc`, a wlroots-based **Wayland**
compositor, not X11. There's an X11 socket at `:0`, but it's unused — if a
GUI app is launched with only `DISPLAY` set, nothing visible renders (GTK
needs `WAYLAND_DISPLAY` present to prefer the real Wayland session). Launch
with:

```bash
WAYLAND_DISPLAY=wayland-0 XDG_RUNTIME_DIR=/run/user/1000 /path/to/ft8af
```

and screenshot with `grim` (not `scrot`, which only captures the blank X11
socket here). `xdotool`/`wmctrl` aren't installed on a stock image.

When driving this over SSH, don't `pkill -f <absolute binary path>` — the
remote shell's own command line contains that same path substring, so
`pkill -f` matches and kills the invoking SSH session itself. Use `pkill -x
<process-name>` (exact `comm` match) instead.

## Not yet done

- This recipe hasn't been ported into `.github/workflows/desktop.yml`'s
  `build-debian-armhf` CI job, which still has an earlier, non-working
  multiarch-install attempt.
- The ~500-package sysroot used during development was assembled
  iteratively while still chasing undefined-symbol errors one at a time,
  before `-Wl,--allow-shlib-undefined` turned out to make most of it
  unnecessary; a much smaller sysroot (just enough for `.pc` resolution and
  the libraries named directly in `-l` flags) would likely suffice and
  build faster.
- No `.deb`/install script yet for either the app binary or the Hamlib
  build — both were deployed by hand over `scp`.
