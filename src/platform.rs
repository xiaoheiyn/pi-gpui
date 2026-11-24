//! Platform pieces gpui 0.2.2 does not provide: AppKit calls on macOS, the
//! freedesktop equivalents on Linux.

// `objc` 0.2 macros probe a `cargo-clippy` feature that modern rustc flags.
#![allow(unexpected_cfgs)]

#[cfg(windows)]
pub use dunce::canonicalize;
#[cfg(unix)]
pub use std::fs::canonicalize;

#[cfg(not(windows))]
use gpui::Window;

pub fn background_command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    let command = std::process::Command::new(program);
    #[cfg(windows)]
    let command = {
        use std::os::windows::process::CommandExt as _;
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

        // 图形应用的子进程不会继承控制台，需阻止系统为它另开窗口。
        let mut command = command;
        command.creation_flags(CREATE_NO_WINDOW);
        command
    };
    command
}

/// Starts dragging the window from our custom title bar.
///
/// Call it on the first mouse move after a mouse down in the title bar, like
/// Zed does. Newer gpui implements this in `Window::start_window_move` with
/// `-[NSWindow performWindowDragWithEvent:]`; in 0.2.2 that method is a no-op
/// on macOS, so we send the same message ourselves.
#[cfg(target_os = "macos")]
pub fn start_window_drag(window: &Window) {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    // `Window` also has an inherent `window_handle()`; ask the raw-window-handle trait.
    let Ok(handle) = HasWindowHandle::window_handle(window) else {
        return;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return;
    };
    let view = appkit.ns_view.as_ptr() as *mut Object;

    // SAFETY: `view` is gpui's live NSView for this window (we are inside one of
    // its event handlers, on the main thread). All messages are standard AppKit
    // API, and null results are checked before use.
    unsafe {
        let ns_window: *mut Object = msg_send![view, window];
        if ns_window.is_null() {
            return;
        }
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        let event: *mut Object = msg_send![app, currentEvent];
        if event.is_null() {
            return;
        }
        let _: () = msg_send![ns_window, performWindowDragWithEvent: event];
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
pub fn start_window_drag(window: &Window) {
    window.start_window_move();
}

/// Moves a file to the Trash, so a deleted session can still be restored.
#[cfg(target_os = "macos")]
pub fn move_to_trash(path: &std::path::Path) -> Result<(), String> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;

    use objc::runtime::{NO, Object};
    use objc::{class, msg_send, sel, sel_impl};

    let path_c = CString::new(path.as_os_str().as_bytes()).map_err(|error| error.to_string())?;
    // SAFETY: standard Foundation calls on the main thread; the out-pointers
    // are valid for the duration of the call and results are checked.
    unsafe {
        let string: *mut Object =
            msg_send![class!(NSString), stringWithUTF8String: path_c.as_ptr()];
        let url: *mut Object = msg_send![class!(NSURL), fileURLWithPath: string];
        let manager: *mut Object = msg_send![class!(NSFileManager), defaultManager];
        let mut error: *mut Object = std::ptr::null_mut();
        let ok: objc::runtime::BOOL = msg_send![
            manager,
            trashItemAtURL: url
            resultingItemURL: std::ptr::null_mut::<*mut Object>()
            error: &mut error
        ];
        if ok != NO {
            return Ok(());
        }
        if error.is_null() {
            return Err("the file could not be moved to the Trash".to_string());
        }
        let description: *mut Object = msg_send![error, localizedDescription];
        let utf8: *const std::os::raw::c_char = msg_send![description, UTF8String];
        Err(std::ffi::CStr::from_ptr(utf8)
            .to_string_lossy()
            .into_owned())
    }
}

/// `gio trash` (GLib, on GNOME and most other desktops), then `kioclient`
/// (KDE), then `trash-put` (trash-cli). Never deletes for good: without any of
/// them the file stays where it is.
#[cfg(target_os = "linux")]
pub fn move_to_trash(path: &std::path::Path) -> Result<(), String> {
    use std::process::Command;

    let attempts: [(&str, &[&str]); 3] = [
        ("gio", &["trash", "--"]),
        ("kioclient", &["move"]),
        ("trash-put", &["--"]),
    ];
    for (program, args) in attempts {
        let mut command = Command::new(program);
        command.args(args).arg(path);
        if program == "kioclient" {
            command.arg("trash:/");
        }
        match command.output() {
            Ok(output) if output.status.success() => return Ok(()),
            Ok(output) => {
                let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(if message.is_empty() {
                    "the file could not be moved to the Trash".to_string()
                } else {
                    message
                });
            }
            Err(_) => continue,
        }
    }
    Err("no trash tool found (install gio, kioclient or trash-cli)".to_string())
}

/// 将会话文件移入 Windows 回收站，保留恢复能力。
#[cfg(windows)]
pub fn move_to_trash(path: &std::path::Path) -> Result<(), String> {
    use widestring::U16CString;
    use windows::Win32::UI::Shell::{
        BHID_Transfer, FOLDERID_RecycleBinFolder, IShellItem, ITransferSource, KF_FLAG_DEFAULT,
        SHCreateItemFromParsingName, SHGetKnownFolderItem,
    };
    use windows::core::PCWSTR;

    let path = std::path::absolute(path).map_err(|error| error.to_string())?;
    let metadata = path.symlink_metadata().map_err(|error| error.to_string())?;
    if !metadata.file_type().is_file() {
        return Err("The session path is not a regular file.".into());
    }
    let path = U16CString::from_os_str(path).map_err(|error| error.to_string())?;

    // SAFETY: GPUI 已初始化调用线程的 COM，路径在调用期间始终有效且以 NUL 结尾。
    let recycle = || unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(path.as_ptr()), None)?;
        let recycle_bin: IShellItem =
            SHGetKnownFolderItem(&FOLDERID_RecycleBinFolder, KF_FLAG_DEFAULT, None)?;
        let transfer: ITransferSource = item.GetParent()?.BindToHandler(None, &BHID_Transfer)?;
        // 回收失败时保留原文件，避免退回到永久删除。
        transfer.RecycleItem(&item, &recycle_bin, 0).map(|_| ())
    };
    recycle().map_err(|error| error.to_string())
}

#[cfg(all(test, windows))]
#[test]
#[ignore = "requires the Windows Recycle Bin on the temporary file's drive"]
fn recycles_a_session_file() {
    use std::io::Write as _;
    use std::os::windows::fs::OpenOptionsExt as _;
    use std::time::{SystemTime, UNIX_EPOCH};

    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("pi-gpui 会话 {stamp}.jsonl"));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(0)
        .open(&path)
        .unwrap();
    file.write_all(b"{}\n").unwrap();

    // SAFETY: 测试线程自行初始化 COM，并在回收接口释放对象后反初始化。
    let (locked_result, result) = unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
        let locked_result = move_to_trash(&path);
        drop(file);
        let result = move_to_trash(&path);
        CoUninitialize();
        (locked_result, result)
    };
    if result.is_err() && path.exists() {
        std::fs::remove_file(&path).unwrap();
    }
    assert!(locked_result.is_err(), "recycled a locked session file");
    assert!(
        result.is_ok(),
        "could not recycle the test session: {result:?}"
    );
    assert!(!path.exists(), "recycled session still exists at {path:?}");
}

#[cfg(windows)]
pub fn reveal_in_file_manager(path: &std::path::Path) {
    use std::os::windows::process::CommandExt as _;
    let mut argument = std::ffi::OsString::from("/select,\"");
    argument.push(path);
    argument.push("\"");
    if let Err(error) = std::process::Command::new("explorer.exe")
        .raw_arg(argument)
        .spawn()
    {
        eprintln!("pi-gpui: could not reveal {}: {error}", path.display());
    }
}

#[cfg(windows)]
pub fn open_in_editor(path: &std::path::Path) {
    if let Err(error) = std::process::Command::new("notepad.exe").arg(path).spawn() {
        eprintln!("pi-gpui: could not open {}: {error}", path.display());
    }
}

/// Shows the file in Finder, or in the file manager on Linux.
#[cfg(unix)]
pub fn reveal_in_file_manager(path: &std::path::Path) {
    use std::process::Command;

    let result = if cfg!(target_os = "macos") {
        Command::new("open").arg("-R").arg(path).spawn().map(drop)
    } else {
        // The freedesktop FileManager1 interface selects the file (Nautilus,
        // Dolphin, Nemo, Thunar…); otherwise just open its folder.
        let uri = file_uri(path);
        let selected = Command::new("gdbus")
            .args([
                "call",
                "--session",
                "--dest",
                "org.freedesktop.FileManager1",
                "--object-path",
                "/org/freedesktop/FileManager1",
                "--method",
                "org.freedesktop.FileManager1.ShowItems",
            ])
            .arg(format!("['{}']", uri.replace('\'', "%27")))
            .arg("")
            .output()
            .is_ok_and(|output| output.status.success());
        if selected {
            Ok(())
        } else {
            let folder = path.parent().unwrap_or(path);
            Command::new("xdg-open").arg(folder).spawn().map(drop)
        }
    };
    if let Err(error) = result {
        eprintln!("pi-gpui: could not reveal {}: {error}", path.display());
    }
}

/// Opens a file in Zed when it is installed, otherwise in the default editor.
#[cfg(unix)]
pub fn open_in_editor(path: &std::path::Path) {
    use std::process::Command;

    let result = if cfg!(target_os = "macos") {
        let zed = std::path::Path::new("/Applications/Zed.app");
        let mut command = Command::new("open");
        if zed.exists() {
            command.arg("-a").arg(zed);
        } else {
            command.arg("-t");
        }
        command.arg(path).spawn().map(drop)
    } else {
        // Zed's CLI is `zed`, or `zeditor` in some distributions' packages.
        match ["zed", "zeditor"]
            .into_iter()
            .find(|program| on_path(program))
        {
            Some(zed) => Command::new(zed).arg(path).spawn().map(drop),
            None => Command::new("xdg-open").arg(path).spawn().map(drop),
        }
    };
    if let Err(error) = result {
        eprintln!("pi-gpui: could not open {}: {error}", path.display());
    }
}

#[cfg(unix)]
fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|directory| directory.join(program).is_file())
    })
}

/// `file://` URI with the bytes that need it percent-encoded.
#[cfg(unix)]
fn file_uri(path: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt as _;

    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            uri.push(byte as char);
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// Files copied in Finder (file URLs on the general pasteboard).
#[cfg(target_os = "macos")]
pub fn clipboard_file_paths() -> Vec<std::path::PathBuf> {
    use std::ffi::{CStr, CString};

    use objc::runtime::{Class, Object, YES};
    use objc::{class, msg_send, sel, sel_impl};

    let Ok(key) = CString::new("NSPasteboardURLReadingFileURLsOnlyKey") else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    // SAFETY: standard AppKit/Foundation calls on the main thread; every
    // returned object is checked for null before use.
    unsafe {
        let pasteboard: *mut Object = msg_send![class!(NSPasteboard), generalPasteboard];
        if pasteboard.is_null() {
            return paths;
        }
        let url_class: *const Class = class!(NSURL);
        let classes: *mut Object = msg_send![class!(NSArray), arrayWithObject: url_class];
        let key: *mut Object = msg_send![class!(NSString), stringWithUTF8String: key.as_ptr()];
        let yes: *mut Object = msg_send![class!(NSNumber), numberWithBool: YES];
        let options: *mut Object =
            msg_send![class!(NSDictionary), dictionaryWithObject: yes forKey: key];
        let urls: *mut Object =
            msg_send![pasteboard, readObjectsForClasses: classes options: options];
        if urls.is_null() {
            return paths;
        }
        let count: usize = msg_send![urls, count];
        for index in 0..count {
            let url: *mut Object = msg_send![urls, objectAtIndex: index];
            let path: *mut Object = msg_send![url, path];
            if path.is_null() {
                continue;
            }
            let utf8: *const std::os::raw::c_char = msg_send![path, UTF8String];
            if !utf8.is_null() {
                paths.push(std::path::PathBuf::from(
                    CStr::from_ptr(utf8).to_string_lossy().into_owned(),
                ));
            }
        }
    }
    paths
}

#[cfg(not(target_os = "macos"))]
pub fn clipboard_file_paths() -> Vec<std::path::PathBuf> {
    Vec::new()
}

/// Asks for one or more folders with the system open panel.
///
/// gpui builds a new `NSOpenPanel` for every prompt, and building one blocks
/// the main thread while AppKit connects to its out-of-process panel service
/// (about half a second the first time, 150 ms after that). We build the panel
/// once and reuse it, so only the first prompt pays that cost.
///
/// Must not be polled while gpui is mid-update: call it from a spawned task.
#[cfg(target_os = "macos")]
pub async fn pick_folders(prompt: &str) -> Result<Option<Vec<std::path::PathBuf>>, String> {
    use std::cell::{Cell, RefCell};
    use std::ffi::{CStr, CString};

    use block::ConcreteBlock;
    use futures::channel::oneshot;
    use objc::runtime::{NO, Object, YES};
    use objc::{class, msg_send, sel, sel_impl};

    thread_local! {
        static PANEL: Cell<*mut Object> = const { Cell::new(std::ptr::null_mut()) };
    }
    const MODAL_RESPONSE_OK: isize = 1;

    let prompt = CString::new(prompt).map_err(|error| error.to_string())?;
    let (done_tx, done_rx) = oneshot::channel::<Option<Vec<std::path::PathBuf>>>();
    // SAFETY: runs on the main thread (gpui's foreground executor). The panel
    // is retained for the life of the app, so the pointer the completion block
    // captures stays valid; all other messages are standard AppKit API.
    unsafe {
        let mut panel = PANEL.with(Cell::get);
        if panel.is_null() {
            panel = msg_send![class!(NSOpenPanel), openPanel];
            if panel.is_null() {
                return Err("NSOpenPanel is unavailable".to_string());
            }
            let _: *mut Object = msg_send![panel, retain];
            PANEL.with(|cell| cell.set(panel));
        }
        let visible: objc::runtime::BOOL = msg_send![panel, isVisible];
        if visible != NO {
            // Already asking; bring it forward instead of starting a second prompt.
            let _: () = msg_send![panel, makeKeyAndOrderFront: std::ptr::null_mut::<Object>()];
            return Ok(None);
        }
        let _: () = msg_send![panel, setCanChooseDirectories: YES];
        let _: () = msg_send![panel, setCanChooseFiles: NO];
        let _: () = msg_send![panel, setAllowsMultipleSelection: YES];
        let _: () = msg_send![panel, setCanCreateDirectories: YES];
        let _: () = msg_send![panel, setResolvesAliases: NO];
        let prompt: *mut Object =
            msg_send![class!(NSString), stringWithUTF8String: prompt.as_ptr()];
        let _: () = msg_send![panel, setPrompt: prompt];

        let done_tx = RefCell::new(Some(done_tx));
        let block = ConcreteBlock::new(move |response: isize| {
            let mut paths = Vec::new();
            if response == MODAL_RESPONSE_OK {
                let urls: *mut Object = msg_send![panel, URLs];
                let count: usize = if urls.is_null() {
                    0
                } else {
                    msg_send![urls, count]
                };
                for index in 0..count {
                    let url: *mut Object = msg_send![urls, objectAtIndex: index];
                    let path: *mut Object = msg_send![url, path];
                    if path.is_null() {
                        continue;
                    }
                    let utf8: *const std::os::raw::c_char = msg_send![path, UTF8String];
                    if !utf8.is_null() {
                        paths.push(std::path::PathBuf::from(
                            CStr::from_ptr(utf8).to_string_lossy().into_owned(),
                        ));
                    }
                }
            }
            if let Some(done_tx) = done_tx.borrow_mut().take() {
                let _ = done_tx.send((response == MODAL_RESPONSE_OK).then_some(paths));
            }
        });
        let block = block.copy();
        let _: () = msg_send![panel, beginWithCompletionHandler: &*block];
    }
    Ok(done_rx.await.unwrap_or(None))
}
