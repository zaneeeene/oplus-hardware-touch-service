// Build script for OPlus hardware touch service

use std::env;
fn main() {
    // Tell cargo to rerun if these files change
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=src/hal.rs");
    println!("cargo:rerun-if-changed=src/xiaomi_touch.rs");
    println!("cargo:rerun-if-changed=src/gesture.rs");
    println!("cargo:rerun-if-changed=src/aod.rs");
    println!("cargo:rerun-if-changed=src/raise_to_wake.rs");
    println!("cargo:rerun-if-changed=src/sysfs.rs");
    println!("cargo:rerun-if-changed=src/procfs.rs");
    println!("cargo:rerun-if-changed=src/config.rs");
    println!("cargo:rerun-if-changed=src/error.rs");

    // Android NDK target detection
    let target = env::var("TARGET").unwrap_or_default();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

    println!("cargo:rustc-env=TARGET_ARCH={}", target_arch);

    // Set up link search paths for Android
    if target.contains("android") {
        let ndk_path = env::var("ANDROID_NDK_HOME")
            .or_else(|_| env::var("NDK_HOME"))
            .or_else(|_| {
                let candidates = [
                    env::var("HOME").map(|home| format!("{home}/android-ndk-r26d")),
                    env::var("HOME").map(|home| format!("{home}/android-ndk-r27c")),
                    Ok("/opt/android-ndk".to_string()),
                    env::var("HOME").map(|home| format!("{home}/Android/Sdk/ndk/26.3.11579264")),
                ];
                candidates
                    .into_iter()
                    .flatten()
                    .find(|path| std::path::Path::new(path).is_dir())
                    .ok_or(env::VarError::NotPresent)
            })
            .unwrap_or_else(|_| "/opt/android-ndk".to_string());

        let api_level = env::var("ANDROID_API_LEVEL").unwrap_or_else(|_| "30".to_string());

        let arch_dir = match target_arch.as_str() {
            "aarch64" => "aarch64",
            "arm" => "arm",
            "x86_64" => "x86_64",
            "x86" => "x86",
            _ => "arm64",
        };

        let lib_path = format!(
            "{}/toolchains/llvm/prebuilt/linux-x86_64/sysroot/usr/lib/{}-linux-android/{}",
            ndk_path, arch_dir, api_level
        );

        println!("cargo:rustc-link-search=native={}", lib_path);
    }

    // Version info
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "1.0.0".to_string());
    let git_hash = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo:rustc-env=BUILD_VERSION={}", version);
    println!("cargo:rustc-env=BUILD_GIT_HASH={}", git_hash);
    println!(
        "cargo:rustc-env=BUILD_TIMESTAMP={}",
        chrono::Utc::now().to_rfc3339()
    );

    // Enable aarch64 target
    if target_arch == "aarch64" {
        println!("cargo:rustc-cfg=target_aarch64");
    }
}
