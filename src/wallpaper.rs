use std::path::Path;

use crate::logger::Logger;

// ── Windows API constants ────────────────────────────────────────────────────

const SPI_SETDESKWALLPAPER: u32 = 0x0014;
const SPIF_UPDATEINIFILE: u32 = 0x0001;
const SPIF_SENDCHANGE: u32 = 0x0002;
const HKEY_CURRENT_USER: isize = -2_147_483_647; // 0x8000_0001u32 as isize
const KEY_READ: u32 = 0x0002_0019;
const REG_SZ: u32 = 1;

// ── FFI declarations (avoids windows-sys dependency) ─────────────────────────

#[link(name = "user32")]
extern "system" {
    fn SystemParametersInfoW(
        uiAction: u32,
        uiParam: u32,
        pvParam: *const u16,
        fWinIni: u32,
    ) -> i32;
}

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenKeyExW(
        hKey: isize,
        lpSubKey: *const u16,
        ulOptions: u32,
        samDesired: u32,
        phkResult: *mut isize,
    ) -> i32;
    fn RegQueryValueExW(
        hKey: isize,
        lpValueName: *const u16,
        lpReserved: *const u32,
        lpType: *mut u32,
        lpData: *mut u8,
        lpcbData: *mut u32,
    ) -> i32;
    fn RegCloseKey(hKey: isize) -> i32;
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Encode a Rust string as a null-terminated UTF-16 `Vec`.
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Normalise a Windows path string for case-insensitive comparison.
pub fn normalize_path(path: &str) -> String {
    path.to_lowercase()
        .replace('/', "\\")
        .trim_start_matches(r"\\?\")
        .to_string()
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Read the current desktop wallpaper path from the registry.
pub fn get_current_wallpaper() -> Option<String> {
    unsafe {
        let mut hkey: isize = 0;
        let subkey = to_wide(r"Control Panel\Desktop");

        if RegOpenKeyExW(HKEY_CURRENT_USER, subkey.as_ptr(), 0, KEY_READ, &mut hkey) != 0 {
            return None;
        }

        let value_name = to_wide("WallPaper");
        let mut buf = vec![0u16; 260];
        let mut buf_size = (buf.len() * 2) as u32;
        let mut value_type: u32 = 0;

        let result = RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null(),
            &mut value_type,
            buf.as_mut_ptr() as *mut u8,
            &mut buf_size,
        );

        RegCloseKey(hkey);

        if result != 0 || value_type != REG_SZ {
            return None;
        }

        let len = buf_size as usize / 2;
        let s = if len > 0 && buf[len - 1] == 0 {
            String::from_utf16_lossy(&buf[..len - 1])
        } else {
            String::from_utf16_lossy(&buf[..len])
        };

        if s.is_empty() { None } else { Some(s) }
    }
}

/// Set the desktop wallpaper and verify the change via the registry.
pub fn set_wallpaper(image_path: &Path, logger: &mut Logger) -> bool {
    let abs_path = std::fs::canonicalize(image_path)
        .unwrap_or_else(|_| image_path.to_path_buf());
    let abs_str = abs_path.to_string_lossy();
    // canonicalize() produces \\?\ prefix on Windows – strip it for the API
    let clean = abs_str.strip_prefix(r"\\?\").unwrap_or(&abs_str);
    let wide = to_wide(clean);

    let result = unsafe {
        SystemParametersInfoW(
            SPI_SETDESKWALLPAPER,
            0,
            wide.as_ptr(),
            SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
        )
    };

    if result == 0 {
        logger.log("SystemParametersInfoW returned False");
        return false;
    }

    std::thread::sleep(std::time::Duration::from_millis(500));

    // Verify
    if let Some(current) = get_current_wallpaper() {
        let current_norm = normalize_path(&current);
        let target_norm = normalize_path(clean);

        if current_norm == target_norm {
            logger.log("Wallpaper changed and verified");
            true
        } else {
            logger.log(&format!(
                "Wallpaper path mismatch. Expected: {clean}, Current: {current}"
            ));
            false
        }
    } else {
        logger.log("Wallpaper changed (unable to verify via registry)");
        true
    }
}
