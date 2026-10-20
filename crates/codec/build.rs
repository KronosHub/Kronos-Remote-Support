//! Hardware encoding (ADR 0058) is built in when `FFMPEG_DIR` points at the minimal FFmpeg
//! made by `tools/ffmpeg/build.sh`; without it the crate has the software encoder only.

fn main() {
    println!("cargo::rustc-check-cfg=cfg(kronos_hw)");
    println!("cargo::rerun-if-env-changed=FFMPEG_DIR");
    println!("cargo::rerun-if-changed=native/hwenc.c");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let Some(dir) = std::env::var_os("FFMPEG_DIR").map(std::path::PathBuf::from) else {
        return;
    };
    let (libs, system): (&[&str], &[&str]) = match target_os.as_str() {
        "macos" => (
            &["avcodec", "avutil"],
            &["VideoToolbox", "CoreMedia", "CoreVideo", "CoreFoundation"],
        ),
        "windows" => (
            &["avcodec", "swscale", "avutil"],
            &[
                "mfplat", "mfuuid", "ole32", "strmiids", "user32", "bcrypt", "winmm",
            ],
        ),
        _ => return,
    };
    cc::Build::new()
        .file("native/hwenc.c")
        .include(dir.join("include"))
        .warnings(true)
        .compile("kronos_hwenc");
    println!(
        "cargo::rustc-link-search=native={}",
        dir.join("lib").display()
    );
    for lib in libs {
        println!("cargo::rustc-link-lib=static={lib}");
    }
    for name in system {
        if target_os == "macos" {
            println!("cargo::rustc-link-lib=framework={name}");
        } else {
            println!("cargo::rustc-link-lib={name}");
        }
    }
    println!("cargo::rustc-cfg=kronos_hw");
}
