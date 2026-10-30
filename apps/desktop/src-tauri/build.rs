fn main() {
    // Windows: the DLLs this program imports are looked up in System32 only, never next to
    // the program (Quick Support runs from Downloads and asks to be elevated: a DLL planted
    // there would run as administrator).
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
    }
    tauri_build::build()
}
