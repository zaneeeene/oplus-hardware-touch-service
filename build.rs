use std::env;

fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-env-changed=ANDROID_NDK_HOME");
    println!("cargo:rerun-if-env-changed=NDK_HOME");
    println!("cargo:rerun-if-env-changed=ANDROID_API_LEVEL");

    let target = env::var("TARGET").unwrap_or_default();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();

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
}
