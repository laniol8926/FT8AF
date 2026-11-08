#!/usr/bin/env bash
# Launcher for FT8AF on Linux.
#
# Historically this LD_PRELOADed a newer Hamlib build to get QRP Labs QMX
# support (the distro's libhamlib.so.4 is 4.5.4, no QMX support at all; a
# separately-built 4.7.1 at /usr/local/lib does have it, same soname, so
# whichever the dynamic linker resolves first wins for any process that
# doesn't override the search path). That env var is inherited by every
# child process a Tauri app spawns, including WebKitGTK's own
# out-of-process WebProcess/NetworkProcess helpers, which have nothing to do
# with Hamlib and should never load it -- confirmed directly that this left
# the webview rendering nothing (WebKitWebProcess alive, never painting),
# while the exact same binary without the env var rendered fine.
#
# Fixed at the root instead: rig.rs's load_hamlib() now tries
# /usr/local/lib/libhamlib.so.4 directly as its first dlopen candidate (see
# PREFERRED_LINUX_HAMLIB), so this script no longer needs to do anything
# beyond finding and exec-ing the binary. Kept as a thin wrapper rather than
# removed outright so the dev-tree-vs-PATH fallback below still has a home.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEV_BIN="$SCRIPT_DIR/../src-tauri/target/release/ft8af"

# Prefer the dev tree's build when this is run from a checkout, but fall back
# to an installed ft8af on PATH -- the .deb/AppImage CI produces puts it at
# /usr/bin/ft8af, and this script is meant to be copied out of the repo.
if [ -x "$DEV_BIN" ]; then
	BIN="$DEV_BIN"
elif BIN="$(command -v ft8af)"; then
	:
else
	echo "error: no ft8af binary found -- looked at $DEV_BIN and on PATH." >&2
	echo "       Build it with 'npm run tauri build' or install the package." >&2
	exit 1
fi

# WebKitGTK's DMABUF renderer can lose its framebuffer when the window loses
# focus (or is unmapped/restored) on some GPU drivers -- confirmed directly
# on this board's software/embedded GL: the webview goes solid light-grey
# (GTK's own background showing through, not a crash -- the process stays
# alive) and never repaints itself, even after refocusing or clicking back
# into it. This is a known WebKitGTK/Tauri issue, most commonly reported on
# NVIDIA but not specific to it; the standard workaround is to disable just
# the DMABUF path (see https://v2.tauri.app/develop/debug/linux-graphics/),
# which keeps most of accelerated compositing and fixed it here. Set
# unconditionally rather than gated on hardware detection -- it's a no-op
# correctness cost on setups that were never hitting the bug, and the
# alternative (WEBKIT_DISABLE_COMPOSITING_MODE, disabling acceleration
# entirely) is a strictly bigger hammer to reach for only if this stops
# being enough on some other GPU/driver combination.
export WEBKIT_DISABLE_DMABUF_RENDERER=1

exec "$BIN" "$@"
