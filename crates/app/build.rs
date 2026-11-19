// libcef.so is copied next to the executable by scripts/build.sh. Without an rpath the
// dynamic loader would not look there and the app would fail to start. The Chromium helper
// processes are this same executable, so they get the same rpath.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo:rustc-link-arg-bins=-Wl,-rpath,$ORIGIN");
    }
}
