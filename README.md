# android-egui-ext

A standalone Rust application for Android that renders an immediate mode GUI ([egui](https://github.com/emilk/egui)) directly to a native window.

**⚠️ Experimental / Research Project**

Unlike standard Android development, this project does **not** use the standard Activity lifecycle (APK). Instead, it runs as a standalone executable (binary) that:
1.  Manually loads the Android Runtime (`libandroid_runtime.so`) using `xdl`.
2.  Spins up a Java VM (`JNI_CreateJavaVM`).
3.  Initializes the Android application context via JNI.
4.  Creates a Native Window / Surface.
5.  Renders `egui` via `egui_glow` (OpenGL ES).

This approach allows for running full GUI applications on Android outside the standard application framework, which can be useful for system-level tools, research, or embedded contexts.

## Prerequisites

*   **Rust**: Stable toolchain.
*   **Android NDK**: Required for compilation. You should have the NDK installed (e.g., via Android Studio or command line tools).
*   **ADB**: Android Debug Bridge to push and run the binary.
*   **Rooted Device (Recommended)**: While the application might run as a shell user depending on permissions, `su` access is often required for interacting with certain system libraries or surfaces.

## Build Instructions

> [!NOTE]
> Building on Windows is not supported yet, but PRs are welcome!

1.  **Configure the Build Environment**:
    Run `config.sh` to generate the `.cargo/config.toml` file. This script detects your NDK installation and sets up the correct linker and target architecture.

    ```sh
    # Usage: ./config.sh <profile> <arch>
    # Profiles: dev, debug
    # Archs: arm64-v8a, armeabi-v7a, x86, x86_64

    ./config.sh dev arm64-v8a
    ```

    *If NDK is not found automatically, set `ANDROID_HOME` or `ANDROID_NDK_HOME` environment variables.*

2.  **Build the Project**:
    Standard cargo build command.

    ```sh
    cargo build
    ```

## Run Instructions

The project includes a custom runner (`runner.sh`) configured by `config.sh`. This runner pushes the compiled binary to `/data/local/tmp` on your connected Android device and executes it.

1.  **Connect your device via ADB**.
2.  **Run with Cargo**:

    ```sh
    # Standard run (as shell user)
    cargo run

    # Run as ROOT (recommended/required for full functionality)
    USE_SU=1 cargo run
    ```

    *Note: The binary is pushed to `/data/local/tmp/android-egui-ext`.*

## Architecture

*   **`src/main.rs`**: Entry point. Orchestrates the runtime loading, VM creation, and render loop.
*   **`src/android/runtime.rs`**: Uses `xdl-rs` to dynamically load `libandroid_runtime.so`, resolve symbols (like `JNI_CreateJavaVM`), and patch internal structures (`AndroidRuntime::mJavaVM`).
*   **`src/renderer.rs`**: Handles EGL context creation and `egui_glow` integration.
*   **`src/bridge.rs`**: JNI bridge to interact with Java classes (e.g., for creating the native window).
*   **`xdl-rs/`**: Rust bindings for [xdl](https://github.com/hexhacking/xdl), used for advanced dynamic linking.

## Status

*   ✅ **Rendering**: Renders the egui demo window.
*   ✅ **JNI**: Successfully creates JVM and calls Java methods.
*   ☑️ **Input**: Touch/Input handling is currently **experimental**. Interaction with the GUI is possible but there's a lot of bug.

## Compatibility

This project has been tested and confirmed to work on:
*   Android 13 (Waydroid)
*   Android 14
*   Android 16

## Credits

*   Inspired by and references techniques from [AndroidImgui](https://github.com/enenH/AndroidImgui).