use windows::{
    core::w,
    Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
        HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ,
    },
};

const RUN_KEY: windows::core::PCWSTR =
    w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE_NAME: windows::core::PCWSTR = w!("RSwitcher");

/// Check whether the app's autostart entry exists in the current user's Run key.
pub fn is_enabled() -> bool {
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            0u32,
            KEY_READ,
            &mut key,
        )
        .ok()
        .is_err()
        {
            return false;
        }
        // NULL data pointer = just test for existence, don't read value.
        let exists = RegQueryValueExW(key, VALUE_NAME, None, None, None, None)
            .ok()
            .is_ok();
        let _ = RegCloseKey(key);
        exists
    }
}

/// Write or delete the autostart registry value.
///
/// Value path:  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
/// Value name:  `RSwitcher`
/// Value type:  `REG_SZ` (UTF-16 path to the current executable)
pub fn set_enabled(enable: bool) {
    let Some(exe) = std::env::current_exe().ok() else { return };

    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            0u32,
            KEY_WRITE,
            &mut key,
        )
        .ok()
        .is_err()
        {
            return;
        }

        if enable {
            // REG_SZ must be UTF-16 LE with a null terminator.
            let wide: Vec<u16> = exe
                .to_string_lossy()
                .encode_utf16()
                .chain([0u16])
                .collect();
            let bytes: Vec<u8> = wide.iter().flat_map(|w| w.to_le_bytes()).collect();
            let _ = RegSetValueExW(key, VALUE_NAME, 0, REG_SZ, Some(bytes.as_slice()));
        } else {
            let _ = RegDeleteValueW(key, VALUE_NAME);
        }

        let _ = RegCloseKey(key);
    }
}
