//! One app per user on Linux and Windows. macOS routes `pi-gpui://` links and reopening to
//! the running app by itself; on Linux every launch (from the dock, a
//! `.desktop` link handler or a terminal) is a new process. The first one
//! listens on a socket in `$XDG_RUNTIME_DIR`; later ones hand it their link
//! and exit.

#[cfg(windows)]
use interprocess::local_socket::{GenericNamespaced, ListenerOptions, Stream, prelude::*};
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(unix)]
use std::path::PathBuf;
use std::time::Duration;

/// Sent when the app is launched again without a link: bring it forward.
pub const ACTIVATE: &str = "activate";

/// Hands `message` to an app that is already running. `true` when it took
/// it, so this process should exit.
pub fn forward(message: &str) -> bool {
    #[cfg(unix)]
    let Some(mut stream) = socket_path().and_then(|path| UnixStream::connect(path).ok()) else {
        return false;
    };
    #[cfg(windows)]
    let Some(mut stream) = pipe_name().and_then(|name| Stream::connect(name).ok()) else {
        return false;
    };
    stream.write_all(format!("{message}\n").as_bytes()).is_ok()
}

/// Listens for later launches; each message goes to `deliver` on a
/// background thread.
pub fn listen(deliver: impl Fn(String) + Send + 'static) {
    #[cfg(unix)]
    let listener = {
        let Some(path) = socket_path() else { return };
        // 上一进程退出后，Unix 域套接字的路径会残留。
        let _ = std::fs::remove_file(&path);
        UnixListener::bind(path)
    };
    #[cfg(windows)]
    let listener = {
        let Some(name) = pipe_name() else { return };
        use interprocess::os::windows::{
            local_socket::ListenerOptionsExt as _, security_descriptor::SecurityDescriptor,
        };
        let owner_only = SecurityDescriptor::deserialize(widestring::u16cstr!("D:P(A;;GA;;;OW)"));
        owner_only.and_then(|descriptor| {
            ListenerOptions::new()
                .name(name)
                .security_descriptor(descriptor)
                .create_sync()
        })
    };
    let listener = match listener {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("pi-gpui: single-instance listener failed: {error}");
            return;
        }
    };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            #[cfg(unix)]
            if stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .is_err()
            {
                continue;
            }
            #[cfg(windows)]
            if stream.set_nonblocking(true).is_err() {
                continue;
            }
            let mut input = BufReader::new(stream).take(128 * 1024);
            let mut bytes = Vec::new();
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            loop {
                match input.read_until(b'\n', &mut bytes) {
                    Ok(_) => break,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
            if let Some(bytes) = bytes.strip_suffix(b"\n")
                && let Ok(message) = std::str::from_utf8(bytes)
                && !message.is_empty()
            {
                deliver(message.to_owned());
            }
        }
    });
}

#[cfg(unix)]
fn socket_path() -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && path.is_dir())?;
    // Development builds keep to themselves, so `cargo run` works while the
    // installed app is open.
    Some(runtime.join(if cfg!(debug_assertions) {
        "pi-gpui-dev.sock"
    } else {
        "pi-gpui.sock"
    }))
}

#[cfg(windows)]
fn pipe_name() -> Option<interprocess::local_socket::Name<'static>> {
    use std::hash::{DefaultHasher, Hash as _, Hasher as _};
    let mut hash = DefaultHasher::new();
    crate::sessions::app_data_directory()?.hash(&mut hash);
    cfg!(debug_assertions).hash(&mut hash);
    format!("pi-gpui-{:016x}", hash.finish())
        .to_ns_name::<GenericNamespaced>()
        .ok()
}
