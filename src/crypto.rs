use anyhow::{Context, Result};

/// Cross-platform credential encryption.
///
/// On Windows, uses DPAPI (CryptProtectData/CryptUnprotectData) to encrypt
/// secrets to the current user's profile. The encrypted blob is Base64-encoded
/// for storage in JSON.
///
/// On non-Windows platforms, uses a simple XOR obfuscation with a fixed key
/// (since there's no secure per-user keystore without external deps). This
/// is NOT cryptographically secure — it only prevents casual reading of the
/// config file. A proper keyring integration would be better but requires
/// additional dependencies.
pub fn encrypt_password(plaintext: &str) -> Result<String> {
    if plaintext.is_empty() {
        return Ok(String::new());
    }

    #[cfg(target_os = "windows")]
    {
        encrypt_windows(plaintext)
    }

    #[cfg(not(target_os = "windows"))]
    {
        encrypt_fallback(plaintext)
    }
}

/// Decrypt a password that was encrypted with [`encrypt_password`].
pub fn decrypt_password(ciphertext: &str) -> Result<String> {
    if ciphertext.is_empty() {
        return Ok(String::new());
    }

    #[cfg(target_os = "windows")]
    {
        decrypt_windows(ciphertext)
    }

    #[cfg(not(target_os = "windows"))]
    {
        decrypt_fallback(ciphertext)
    }
}

#[cfg(target_os = "windows")]
fn encrypt_windows(plaintext: &str) -> Result<String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{BOOL, HLOCAL, LocalFree};
    use windows_sys::Win32::Security::Cryptography::{
        CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    use windows_sys::core::PCWSTR;

    // Convert plaintext to UTF-16LE bytes (as expected by DPAPI)
    let data: Vec<u16> = OsStr::new(plaintext).encode_wide().collect();
    let mut data_in = CRYPT_INTEGER_BLOB {
        cbData: (data.len() * 2) as u32,
        pbData: data.as_ptr() as *mut u8,
    };

    // Optional entropy (None = use user's logon credentials only)
    let mut entropy_blob: Option<CRYPT_INTEGER_BLOB> = None;

    let mut data_out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };

    // Description shown in the prompt (not used with UI_FORBIDDEN)
    let desc: Vec<u16> = OsStr::new("Jenkins Tray credential").encode_wide().collect();

    let result = unsafe {
        windows_sys::Win32::Security::Cryptography::CryptProtectData(
            &mut data_in,
            PCWSTR(desc.as_ptr()),
            entropy_blob.as_mut().map_or(std::ptr::null_mut(), |e| e),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut data_out,
        )
    };

    if result == BOOL(0) {
        anyhow::bail!("CryptProtectData failed: {}", std::io::Error::last_os_error());
    }

    // Copy the encrypted blob to a Rust Vec and free the DPAPI allocation
    let encrypted = unsafe {
        let slice = std::slice::from_raw_parts(data_out.pbData, data_out.cbData as usize);
        slice.to_vec()
    };

    // Free the DPAPI-allocated memory
    unsafe {
        LocalFree(data_out.pbData as HLOCAL);
    }

    // Base64 encode for JSON storage
    use base64::Engine;
    Ok(base64::engine::general_purpose::STANDARD.encode(encrypted))
}

#[cfg(target_os = "windows")]
fn decrypt_windows(ciphertext: &str) -> Result<String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::{BOOL, HLOCAL, LocalFree};
    use windows_sys::Win32::Security::Cryptography::{
        CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    // Base64 decode
    use base64::Engine;
    let encrypted = base64::engine::general_purpose::STANDARD
        .decode(ciphertext)
        .context("Failed to decode Base64 credential")?;

    let mut data_in = CRYPT_INTEGER_BLOB {
        cbData: encrypted.len() as u32,
        pbData: encrypted.as_ptr() as *mut u8,
    };

    let mut data_out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };

    let result = unsafe {
        windows_sys::Win32::Security::Cryptography::CryptUnprotectData(
            &mut data_in,
            std::ptr::null_mut(), // ppwszDataDescr (we don't need the description)
            std::ptr::null_mut(), // optional entropy
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut data_out,
        )
    };

    if result == BOOL(0) {
        anyhow::bail!("CryptUnprotectData failed: {}", std::io::Error::last_os_error());
    }

    // Convert the decrypted UTF-16LE bytes back to String
    let decrypted = unsafe {
        let slice = std::slice::from_raw_parts(data_out.pbData as *const u16, (data_out.cbData / 2) as usize);
        OsString::from_wide(slice).to_string_lossy().into_owned()
    };

    // Free the DPAPI-allocated memory
    unsafe {
        LocalFree(data_out.pbData as HLOCAL);
    }

    Ok(decrypted)
}

#[cfg(not(target_os = "windows"))]
fn encrypt_fallback(plaintext: &str) -> Result<String> {
    use base64::Engine;
    // Simple XOR obfuscation with a fixed key (NOT secure, just obscures)
    const KEY: &[u8] = b"JenkinsTrayObfuscationKey123456789";
    let mut result = Vec::with_capacity(plaintext.len());
    for (i, b) in plaintext.bytes().enumerate() {
        result.push(b ^ KEY[i % KEY.len()]);
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(result))
}

#[cfg(not(target_os = "windows"))]
fn decrypt_fallback(ciphertext: &str) -> Result<String> {
    use base64::Engine;
    const KEY: &[u8] = b"JenkinsTrayObfuscationKey123456789";
    let encrypted = base64::engine::general_purpose::STANDARD
        .decode(ciphertext)
        .context("Failed to decode Base64 credential")?;
    let mut result = Vec::with_capacity(encrypted.len());
    for (i, b) in encrypted.iter().enumerate() {
        result.push(b ^ KEY[i % KEY.len()]);
    }
    Ok(String::from_utf8(result).context("Decrypted data is not valid UTF-8")?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let plaintext = "test_password_123!@#";
        let encrypted = encrypt_password(plaintext).expect("encrypt failed");
        assert!(!encrypted.is_empty());
        assert_ne!(encrypted, plaintext);
        let decrypted = decrypt_password(&encrypted).expect("decrypt failed");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn empty_string() {
        assert_eq!(encrypt_password("").unwrap(), "");
        assert_eq!(decrypt_password("").unwrap(), "");
    }

    #[test]
    fn unicode_password() {
        let plaintext = "pässwörd_🔑_中文";
        let encrypted = encrypt_password(plaintext).expect("encrypt failed");
        let decrypted = decrypt_password(&encrypted).expect("decrypt failed");
        assert_eq!(decrypted, plaintext);
    }
}