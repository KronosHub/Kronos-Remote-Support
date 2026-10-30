fn main() {
    // Imported DLLs from System32 only, never from the folder of this self-elevating tool
    // (often Downloads): see apps/desktop/src-tauri/build.rs.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/DEPENDENTLOADFLAG:0x800");
    }
}
