use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=external/xdl/CMakeLists.txt");
    println!("cargo:rerun-if-changed=external/xdl/include/xdl.h");

    let target = env::var("TARGET").unwrap();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Build xdl using cmake
    let mut cfg = cmake::Config::new("external/xdl");

    // Explicitly set the compilers using the environment variables set by .cargo/config.toml
    if let Ok(cc) = env::var("CC") {
        cfg.define("CMAKE_C_COMPILER", cc);
    }
    if let Ok(cxx) = env::var("CXX") {
        cfg.define("CMAKE_CXX_COMPILER", cxx);
    }

    // Android-specific configuration
    if target.contains("android") {
        cfg.define("CMAKE_SYSTEM_NAME", "Android");

        // Determine Android ABI
        let abi = match target.as_str() {
            t if t.contains("aarch64") => "arm64-v8a",
            t if t.contains("armv7") => "armeabi-v7a",
            t if t.contains("x86_64") => "x86_64",
            t if t.contains("i686") => "x86",
            _ => "",
        };

        if !abi.is_empty() {
            cfg.define("ANDROID_ABI", abi);
        }

        // Setup NDK toolchain if available
        if let Ok(ndk) = env::var("ANDROID_NDK_HOME") {
            let ndk_path = PathBuf::from(ndk);
            let toolchain = ndk_path.join("build/cmake/android.toolchain.cmake");
            if toolchain.exists() {
                cfg.define("CMAKE_TOOLCHAIN_FILE", toolchain);
            }
            cfg.define("ANDROID_NDK", &ndk_path);
        }
    }

    let dst = cfg.build_target("xdl").build();

    println!("cargo:rustc-link-search=native={}/build", dst.display());
    println!("cargo:rustc-link-lib=static=xdl");

    // Generate bindings
    let mut builder = bindgen::Builder::default()
        .header("external/xdl/include/xdl.h")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .size_t_is_usize(true)
        .clang_arg(format!("--target={}", target));

    // If we are building for Android, we might need to point bindgen to the right sysroot
    if let (true, Ok(ndk)) = (target.contains("android"), env::var("ANDROID_NDK_HOME")) {
        let ndk_path = PathBuf::from(ndk);
        // This is a bit of a heuristic for NDK layout
        let sysroot = ndk_path.join("toolchains/llvm/prebuilt/linux-x86_64/sysroot");
        if sysroot.exists() {
            builder = builder.clang_arg(format!("--sysroot={}", sysroot.display()));
        }
    }

    let bindings = builder.generate().expect("Unable to generate bindings");

    bindings
        .write_to_file(out_dir.join("xdl.h.rs"))
        .expect("Couldn't write bindings!");
}
