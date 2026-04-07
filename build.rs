use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn contains_aeron_library(path: &Path, stem: &str) -> bool {
    ["a", "so", "dylib", "lib"]
        .into_iter()
        .map(|ext| path.join(format!("lib{stem}.{ext}")))
        .chain(std::iter::once(path.join(format!("{stem}.lib"))))
        .any(|candidate| candidate.is_file())
}

fn is_aeron_lib_dir(path: &Path) -> bool {
    path.is_dir()
        && contains_aeron_library(path, "aeron")
        && contains_aeron_library(path, "aeron_driver")
}

fn find_prebuilt_aeron_lib_dir() -> Option<PathBuf> {
    let path = env::var("AERON_LIB_DIR").ok()?;
    let path = PathBuf::from(path);
    if is_aeron_lib_dir(&path) {
        Some(path)
    } else {
        panic!(
            "AERON_LIB_DIR does not point to a directory containing official Aeron libraries (expected libaeron + libaeron_driver): {}",
            path.display()
        );
    }
}

fn ensure_cmake_available() {
    let cmake = env::var("CMAKE").unwrap_or_else(|_| "cmake".to_string());
    let status = Command::new(&cmake)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    if status.is_ok() {
        return;
    }

    panic!(
        "Could not execute `{cmake}` while building Aeron from source. Install CMake, set CMAKE=/path/to/cmake, or set AERON_LIB_DIR=/path/to/prebuilt/aeron/lib."
    );
}

fn emit_link_directives(lib_dir: &Path) {
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=aeron");
    println!("cargo:rustc-link-lib=aeron_driver");

    if cfg!(any(target_os = "macos", target_os = "linux")) {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib_dir.display());
    }
}

fn is_aeron_source_dir(path: &Path) -> bool {
    path.join("CMakeLists.txt").is_file()
        && path.join("aeron-client/src/main/c/aeronc.h").is_file()
        && path.join("aeron-driver/src/main/c/aeronmd.h").is_file()
}

fn find_aeron_source_dir() -> PathBuf {
    if let Ok(path) = env::var("AERON_SOURCE_DIR") {
        let path = PathBuf::from(path);
        if is_aeron_source_dir(&path) {
            return path;
        }
        panic!(
            "AERON_SOURCE_DIR does not point to an official Aeron source checkout: {}",
            path.display()
        );
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let candidates = [
        manifest_dir.join("vendor/aeron"),
        PathBuf::from("/tmp/aeron-official"),
        PathBuf::from("/Users/matt/projects/active/aeron"),
    ];

    for candidate in candidates {
        if is_aeron_source_dir(&candidate) {
            return candidate;
        }
    }

    panic!(
        "Could not locate an official Aeron source checkout. Set AERON_SOURCE_DIR=/path/to/aeron or place a checkout under vendor/aeron."
    );
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=AERON_LIB_DIR");
    println!("cargo:rerun-if-env-changed=AERON_SOURCE_DIR");
    println!("cargo:rerun-if-env-changed=CMAKE");

    if let Some(lib_dir) = find_prebuilt_aeron_lib_dir() {
        emit_link_directives(&lib_dir);
        return;
    }

    let aeron_source_dir = find_aeron_source_dir();
    ensure_cmake_available();
    let dst = cmake::Config::new(&aeron_source_dir)
        .define("BUILD_AERON_ARCHIVE_API", "OFF")
        .define("AERON_TESTS", "OFF")
        .define("AERON_BUILD_SAMPLES", "OFF")
        .define("AERON_BUILD_DOCUMENTATION", "OFF")
        .define("AERON_INSTALL_TARGETS", "ON")
        .profile("Release")
        .build();

    emit_link_directives(&dst.join("lib"));
}
