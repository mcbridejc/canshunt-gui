# CANShunt GUI

Tauri desktop application for configuring and monitoring CANShunt devices over an existing Linux SocketCAN interface or directly over the board's `gs_usb` USB interface. CANopen protocol handling is provided by the adjacent `zencan-client` crate.

## Development

Install the [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/), then run:

```sh
npm install
npm run tauri dev
```

SocketCAN interfaces must already be configured and up. Direct USB access may require suitable device permissions; the application detaches and restores an attached kernel driver while it owns the device.

## Building desktop binaries


Install a current Node.js LTS release, Rust stable, and the operating system's Tauri prerequisites. From `canshunt-gui`, install the locked frontend dependencies and create a release build:

```sh
npm ci
npm run tauri build
```

The packaged artifacts are written below `src-tauri/target/release/bundle/`. The unpackaged executable is in `src-tauri/target/release/`.

### Linux

Install the Linux packages listed in the Tauri prerequisites for your distribution, including the WebKitGTK development libraries. Then run the common build commands above. With the current `bundle.targets` setting, Tauri builds the bundle formats supported by the host, such as Debian packages, RPMs, and AppImages.

SocketCAN support is Linux-only. Direct `gs_usb` access is available through libusb and may require an appropriate udev rule or root privileges.

### Windows

Install Rust using the MSVC toolchain, Microsoft C++ Build Tools with Desktop development with C++, and Microsoft Edge WebView2. Run the common build commands from PowerShell. Tauri generates Windows installers such as `.msi` and NSIS `.exe` files. Building MSI packages also requires the Windows VBSCRIPT optional feature.

The CANShunt must use a libusb-compatible Windows driver, such as WinUSB, for direct USB access.

### macOS

Install the Xcode command-line tools:

```sh
xcode-select --install
```

Run the common build commands to build for the Mac's native architecture. To make one application containing both Apple Silicon and Intel binaries, install both Rust targets and request Tauri's universal target:

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
npm run tauri build -- --target universal-apple-darwin
```
