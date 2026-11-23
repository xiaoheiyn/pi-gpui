# Windows 11 x64 preview

Extract the whole ZIP to a folder and double-click `Pi GPUI.exe`. Install and authenticate a compatible pi first. Pi GPUI looks in PATH and the default npm global directory. The `PI_EXECUTABLE` environment variable can point to a specific pi installation. Settings and projects are stored under `%LOCALAPPDATA%\pi-gpui`.

To build the preview locally on Windows x64, install the Rust toolchain from `rust-toolchain.toml`, Visual Studio Build Tools with the Desktop development with C++ workload, and a Windows SDK with FXC. Run `rustup target add x86_64-pc-windows-msvc` and `./script/bundle-windows.ps1`. The Windows preview workflow also produces the ZIP as a downloadable build artifact.

Before publishing the preview, test the ZIP on a Windows 11 x64 machine with compatible pi installed. Open a project whose path contains spaces and Chinese characters, send a message using a Chinese input method, and verify its reply. Run two same-named workers, confirm their records separately, then restart the app and reopen both from the parent call. Check the labels and arrow direction in a Chinese Mermaid flowchart. Open the launcher a second time while a window is running and confirm the existing window receives the project.

Deleting a session moves its file to the Windows Recycle Bin. If Windows cannot recycle the file, Pi GPUI shows an error and keeps the file in place.
