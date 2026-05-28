#![cfg(windows)]

use anyhow::{Context, Result};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::DataExchange::AddClipboardFormatListener;

/// WM_CLIPBOARDUPDATE — windows-rs sabit olarak vermiyor, doğrudan yazıyoruz.
pub const WM_CLIPBOARDUPDATE: u32 = 0x031D;

pub fn register_listener(hwnd: HWND) -> Result<()> {
    unsafe { AddClipboardFormatListener(hwnd) }
        .context("AddClipboardFormatListener başarısız")?;
    Ok(())
}

/// Panodaki Unicode metni okur. Pano metin değilse veya başka bir uygulama açmışsa
/// `None` döner; çağıran sessizce geçer.
pub fn read_text() -> Option<String> {
    clipboard_win::get_clipboard_string().ok()
}

/// Panoya metin yazar. UI thread'inden çağrılmalı (Win32 pano owner kuralı).
pub fn write_text(text: &str) -> Result<()> {
    clipboard_win::set_clipboard_string(text)
        .map_err(|e| anyhow::anyhow!("set_clipboard hatası: {:?}", e))
}
