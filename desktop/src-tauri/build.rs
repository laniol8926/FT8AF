// Compile the pure-C FT8 DSP core (kgoba ft8_lib pinned at 6f528128) plus the
// FT8CN GFSK glue into a static `ft8core` lib that the Rust FFI layer links.
//
// We reference the C sources in place inside the Android tree (../../ft8af/...)
// rather than duplicating them, so the desktop port tracks the same DSP as the
// app. This mirrors the standalone build recipe proven by ft8af_glue/test_roundtrip.c.

use std::path::PathBuf;

// Arch-selection helpers (hamlib_arch_subdir / clang_cl_target_flag), shared
// verbatim with the library so their logic is unit-tested by `cargo test`.
// Included rather than `mod`'d because a build script is its own crate root.
include!("src/build_support.rs");

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let target = std::env::var("TARGET").unwrap_or_default();
    // src-tauri -> desktop -> NEXT-FT8CN
    let cpp = manifest.join("../../ft8af/app/src/main/cpp");
    let ft8 = cpp.join("ft8_lib");
    let glue = cpp.join("ft8af_glue");

    let mut build = cc::Build::new();
    build.include(&ft8).include(&glue);

    // ft8_lib uses C99 variable-length arrays (decode.c, monitor.c). MSVC's cl.exe
    // rejects VLAs, so on the windows-msvc target compile with clang-cl instead:
    // it supports VLAs and still emits MSVC-ABI objects that link into the Rust
    // binary — and it's the same compiler family as the Android NDK, keeping the
    // DSP bit-identical. macOS/Linux use the default clang/gcc, which handle VLAs.
    if target.contains("windows-msvc") {
        if let Some(clang_cl) = find_clang_cl() {
            build.compiler(clang_cl);
        }
        // clang-cl defaults to the host architecture. When cross-compiling the
        // 64-bit host down to 32-bit x86 (i686), pass the target triple so the
        // `ft8core` objects are 32-bit and link into the i686 Rust binary; x64
        // keeps clang-cl on its host default (unchanged behavior). The
        // _USE_MATH_DEFINES / C99-VLA path below is arch-independent, so it
        // still compiles under x86.
        if let Some(flag) = clang_cl_target_flag(&target) {
            build.flag(&flag);
        }
        // Force-include the compat shim (provides stpcpy) into every C TU.
        build.include(manifest.join("cbits"));
        build.flag("/FIwin_compat.h");
    }

    // ft8_lib core (matches the file set in test_roundtrip.c's build line).
    for f in [
        "ft8/pack.c",
        "ft8/encode.c",
        "ft8/decode.c",
        "ft8/ldpc.c",
        "ft8/osd.c",
        "ft8/message.c",
        "ft8/text.c",
        "ft8/unpack.c",
        "ft8/crc.c",
        "ft8/constants.c",
        "common/monitor.c",
        "fft/kiss_fft.c",
        "fft/kiss_fftr.c",
    ] {
        build.file(ft8.join(f));
    }
    // FT8CN glue: GFSK waveform synthesis with the trailing `offset` param.
    build.file(glue.join("gfsk.c"));
    // Coherent time-domain signal subtraction (successive interference
    // cancellation) for the "Deep" decode tier's subtract-and-redecode
    // loop -- already shipped on Android, newly wired up on desktop (see
    // engine.rs's decoder thread). Depends on gfsk.c's
    // synth_gfsk_dphi_alloc, already compiled above.
    build.file(glue.join("ft8_subtract.c"));

    // MSVC needs _USE_MATH_DEFINES for M_PI; silence CRT string warnings.
    build.define("_USE_MATH_DEFINES", None);
    build.define("_CRT_SECURE_NO_WARNINGS", None);
    build.warnings(false);

    build.compile("ft8core");

    // On Windows, copy the vendored Hamlib DLLs next to the output exe so the
    // app loads rig support out of the box (no separate Hamlib install). The
    // exe's own directory is first in the DLL search order, so libhamlib-4.dll
    // and its dependencies resolve automatically. This covers `cargo
    // run`/`tauri dev`, but NOT the installed MSI/NSIS bundle -- the Tauri
    // bundler only packages the exe plus whatever is declared under
    // `bundle.resources`, so it never picks up files that just happen to be
    // sitting in target/. `hamlib-dist/` is the resource dir
    // `tauri.windows.conf.json` declares for that purpose; Tauri places
    // declared resources next to the installed exe on Windows, which is the
    // same location `load_hamlib()`'s DLL search already checks first.
    if target.contains("windows") {
        copy_hamlib_dlls(&manifest, &target);
        copy_hamlib_dlls_to_resource_dir(&manifest, &target);
    }

    println!("cargo:rerun-if-changed={}", cpp.display());
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/build_support.rs");
    println!("cargo:rerun-if-changed=hamlib");

    // Build timestamp for Settings -> About's "Build <date>" row (Android's
    // own `settings_build_date_format`/`GeneralVariables.BUILD_DATE`). Pure
    // `std::time`, no extra build-dependency -- formatted at runtime in
    // main.rs with the `chrono` the main crate already depends on.
    let build_timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=FT8AF_BUILD_TIMESTAMP={build_timestamp}");

    // Generate the Tauri build context (reads tauri.conf.json).
    tauri_build::build();
}

/// The vendored DLL set matching the target process architecture (`hamlib/x86`
/// or `hamlib/x64`), falling back to the flat `hamlib/` layout for older
/// checkouts without the split. `None` means nothing to copy (non-Windows, or
/// no `hamlib/` dir at all).
fn hamlib_src_dir(manifest: &std::path::Path, target: &str) -> Option<PathBuf> {
    let hamlib = manifest.join("hamlib");
    if !hamlib.is_dir() {
        return None;
    }
    let arch = hamlib_arch_subdir(target)?;
    let sub = hamlib.join(arch);
    Some(if sub.is_dir() { sub } else { hamlib })
}

fn copy_dlls(src: &std::path::Path, dst: &std::path::Path) {
    if let Ok(entries) = std::fs::read_dir(src) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) == Some("dll") {
                if let Some(name) = p.file_name() {
                    let _ = std::fs::copy(&p, dst.join(name));
                }
            }
        }
    }
}

fn copy_hamlib_dlls(manifest: &std::path::Path, target: &str) {
    // If the arch dir exists but ships no DLLs yet (e.g. the 32-bit set hasn't
    // been vendored), copy nothing — the app still builds and rig control
    // degrades cleanly to "Hamlib unavailable" via load_hamlib()'s fallback.
    let Some(src) = hamlib_src_dir(manifest, target) else {
        return;
    };
    // OUT_DIR = target/<profile>/build/<pkg>/out  -> exe dir is 3 parents up.
    let out_dir = match std::env::var("OUT_DIR") {
        Ok(d) => PathBuf::from(d),
        Err(_) => return,
    };
    let exe_dir = match out_dir.ancestors().nth(3) {
        Some(p) => p.to_path_buf(),
        None => return,
    };
    copy_dlls(&src, &exe_dir);
}

/// Mirrors the arch-correct Hamlib DLLs into a stable `hamlib-dist/` dir next
/// to `tauri.conf.json`, which `tauri.windows.conf.json` declares as a bundle
/// resource. `copy_hamlib_dlls` above puts the DLLs next to the exe for
/// `cargo run`/`tauri dev`, but the Tauri bundler only packages the exe plus
/// declared `bundle.resources` -- it doesn't sweep up arbitrary files sitting
/// in `target/`, so without this step the installed MSI/NSIS app has no DLL
/// next to it and rig control silently needs a separate system Hamlib
/// install. Cleared and repopulated every build so a stale wrong-arch DLL
/// from a previous target can't linger (e.g. a local x64 build followed by a
/// CI x86 cross-build sharing this checkout).
fn copy_hamlib_dlls_to_resource_dir(manifest: &std::path::Path, target: &str) {
    let Some(src) = hamlib_src_dir(manifest, target) else {
        return;
    };
    let dist = manifest.join("hamlib-dist");
    let _ = std::fs::remove_dir_all(&dist);
    if std::fs::create_dir_all(&dist).is_err() {
        return;
    }
    copy_dlls(&src, &dist);
}

/// Locate clang-cl: honor an explicit CLANG_CL env override, otherwise try PATH
/// and the default LLVM install location.
fn find_clang_cl() -> Option<std::ffi::OsString> {
    if let Ok(p) = std::env::var("CLANG_CL") {
        return Some(p.into());
    }
    let default = PathBuf::from(r"C:\Program Files\LLVM\bin\clang-cl.exe");
    if default.exists() {
        return Some(default.into_os_string());
    }
    // Fall back to PATH lookup (cc will resolve a bare name against PATH).
    Some("clang-cl.exe".into())
}

