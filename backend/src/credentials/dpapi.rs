//! Windows DPAPI credential store (ADR 0003): `CryptProtectData` with
//! `CRYPTPROTECT_LOCAL_MACHINE` (machine scope) and app-specific entropy.

use windows_sys::Win32::Foundation::{LocalFree, HLOCAL};
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_LOCAL_MACHINE, CRYPTPROTECT_UI_FORBIDDEN,
    CRYPT_INTEGER_BLOB,
};

use super::{backend_tag, CredentialError, CredentialStore, CONTEXT};

const TAG: &[u8] = b"dpapi1:";

pub struct DpapiStore;

fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(data.len()).unwrap_or(u32::MAX),
        pbData: data.as_ptr().cast_mut(),
    }
}

/// Copy a DPAPI output blob into a Vec and free it with `LocalFree`.
///
/// SAFETY: `out` must have been filled by a successful CryptProtectData /
/// CryptUnprotectData call, which allocates `pbData` with `LocalAlloc`.
unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
    LocalFree(out.pbData as HLOCAL);
    v
}

impl CredentialStore for DpapiStore {
    fn name(&self) -> &'static str {
        "dpapi (machine scope)"
    }

    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, CredentialError> {
        let input = blob(plaintext);
        let entropy = blob(CONTEXT);
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: all pointers reference live, correctly sized buffers for the
        // duration of the call; DPAPI does not retain them.
        let ok = unsafe {
            CryptProtectData(
                &input,
                std::ptr::null(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_LOCAL_MACHINE | CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(CredentialError::Encrypt(format!(
                "CryptProtectData failed: {}",
                std::io::Error::last_os_error()
            )));
        }
        // SAFETY: the call succeeded, so `out` holds a LocalAlloc'd buffer.
        let sealed = unsafe { take(out) };
        Ok([TAG, &sealed].concat())
    }

    fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, CredentialError> {
        let rest = ciphertext
            .strip_prefix(TAG)
            .ok_or_else(|| CredentialError::WrongBackend(backend_tag(ciphertext)))?;
        let input = blob(rest);
        let entropy = blob(CONTEXT);
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: as in `encrypt`.
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(CredentialError::Decrypt);
        }
        // SAFETY: the call succeeded, so `out` holds a LocalAlloc'd buffer.
        Ok(unsafe { take(out) })
    }
}
