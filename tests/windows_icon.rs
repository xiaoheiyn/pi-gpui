#![cfg(windows)]

use widestring::U16CString;
use windows::{
    Win32::UI::{
        Shell::ExtractIconExW,
        WindowsAndMessaging::{DestroyIcon, HICON},
    },
    core::PCWSTR,
};

#[test]
fn executable_has_a_shell_icon() {
    let executable = U16CString::from_str(env!("CARGO_BIN_EXE_pi-gpui"))
        .expect("Cargo's executable path cannot contain a NUL");
    let mut icon = HICON::default();

    // SAFETY: 路径以空字符结尾，请求的单个图标由有效指针接收。
    let count = unsafe { ExtractIconExW(PCWSTR(executable.as_ptr()), 0, Some(&mut icon), None, 1) };
    assert_eq!(count, 1, "the shell must extract the executable icon");
    assert!(!icon.is_invalid(), "the shell must return a valid icon");
    // SAFETY: ExtractIconExW 返回的有效句柄由调用方负责释放。
    unsafe { DestroyIcon(icon) }.expect("release the extracted icon");
}
