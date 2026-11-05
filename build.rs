//! Builds mGBA from the `third_party/mgba` submodule, plus the C shim in
//! `src/emulator.c` that gives the Rust side a small, flat API over it.

use std::path::Path;

fn main() {
    let mgba = Path::new("third_party/mgba");
    assert!(
        mgba.join("CMakeLists.txt").exists(),
        "mGBA's source is missing; run `git submodule update --init`"
    );

    // Only the GBA core and its debugger are needed, without front ends or
    // optional dependencies. mGBA is always built optimized, because the
    // emulator is unusably slow at the debug profile's -O0.
    let out = cmake::Config::new(mgba)
        .define("LIBMGBA_ONLY", "ON")
        .define("M_CORE_GB", "OFF")
        .define("USE_DEBUGGERS", "ON")
        .profile("Release")
        .build_target("mgba")
        .build();
    let build = out.join("build");

    // `mgba/flags.h` is generated into the build tree and records the
    // features mGBA was built with, so the shim sees the same struct layouts.
    cc::Build::new()
        .file("src/emulator.c")
        .include(mgba.join("include"))
        .include(build.join("include"))
        .define("_GNU_SOURCE", None)
        .compile("gbamod_emulator");

    // The shim is linked before mGBA, which it depends on.
    println!("cargo:rustc-link-search=native={}", build.display());
    println!("cargo:rustc-link-lib=static=mgba");
    println!("cargo:rustc-link-lib=m");
    println!("cargo:rustc-link-lib=pthread");

    println!("cargo:rerun-if-changed=src/emulator.c");
    println!("cargo:rerun-if-changed=third_party/mgba");
}
