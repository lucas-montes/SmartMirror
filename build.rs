fn main() {
    // sherpa-onnx is linked statically and carries its own copy of OpenFST/Kaldi.
    // libvosk.so ships a *different* copy with the same symbol names. Without this,
    // the executable exports sherpa's copies and libvosk's static initializers bind
    // to them at load time, which segfaults before `main` runs.
    // Hiding symbols from static archives keeps each library on its own copy.
    #[cfg(target_os = "linux")]
    println!("cargo:rustc-link-arg-bins=-Wl,--exclude-libs,ALL");
}
