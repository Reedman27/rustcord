//! Compiles the C++ shim against Qt6 WebEngine Widgets, found through pkg-config.
use std::process::Command;

fn pkg(args: &[&str]) -> Vec<String> {
    let out = Command::new("pkg-config")
        .args(args)
        .args(["Qt6WebEngineWidgets", "Qt6Widgets", "Qt6Gui", "Qt6Core"])
        .output()
        .expect("pkg-config not found");
    if !out.status.success() {
        panic!(
            "pkg-config could not find Qt6 WebEngine Widgets ({}). Install the Qt6 WebEngine \
             development package (Chimera: qt6-qtwebengine-devel).",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

fn main() {
    println!("cargo:rerun-if-changed=src/shim.cpp");
    let mut b = cc::Build::new();
    b.cpp(true)
        .file("src/shim.cpp")
        .flag("-std=c++17")
        .flag("-fPIC");
    for f in pkg(&["--cflags"]) {
        b.flag(&f);
    }
    b.warnings(true).compile("rustcord_shim");
    for f in pkg(&["--libs"]) {
        if let Some(dir) = f.strip_prefix("-L") {
            println!("cargo:rustc-link-search=native={dir}");
        } else if let Some(lib) = f.strip_prefix("-l") {
            println!("cargo:rustc-link-lib={lib}");
        }
    }
    // The shim is C++: link the C++ standard library the toolchain uses.
    let cxx = if cfg!(target_os = "macos") {
        "c++"
    } else {
        "stdc++"
    };
    if std::env::var("CXXSTDLIB").is_err() {
        println!("cargo:rustc-link-lib={cxx}");
    }
}
