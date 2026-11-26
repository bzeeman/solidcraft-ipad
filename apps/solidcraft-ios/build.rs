//! The main thread's stack. An iPad gives it 1 MB unless the executable asks for more, and
//! building the app (`SolveApp::new`) needs more than that; the simulator runs with a Mac's 8 MB,
//! so the overflow only shows on a real iPad. 16 MB leaves plenty of room. Whether iOS honours
//! this for the main thread is checked on the device (build 1004).

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("ios") {
        println!("cargo:rustc-link-arg-bins=-Wl,-stack_size,0x1000000");
    }
}
