use std::path::Path;

fn main() {
    let config_path = Path::new(".cargo/config.toml");
    if !config_path.exists() {
        panic!(
            "\n\n Error: .cargo/config.toml not found!\n
            Please run ./config.sh [profile] <arch> first to configure the Android environment.\n
            Supported profiles: dev (fast transfer), debug (default)\n
            Supported architectures: arm64-v8a, armeabi-v7a, x86, x86-64\n\n"
        );
    }

    println!("cargo:rerun-if-changed=.cargo/config.toml");
}
