//! Credential Manager: generic credentials, kept on this PC only
//! (`CRED_PERSIST_LOCAL_MACHINE`: they don't roam with the profile).

use std::ffi::c_void;
use std::ptr;

use windows_sys::Win32::Foundation::{GetLastError, ERROR_NOT_FOUND};
use windows_sys::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
    CRED_TYPE_GENERIC,
};

use super::{decode_blob, Secret, SecretError};

/// Shown as the credential's user name in Credential Manager.
const USER: &str = "desktop-assistant";

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn failed(what: &str) -> SecretError {
    // SAFETY: reads the calling thread's last error; no pointers involved.
    let code = unsafe { GetLastError() };
    SecretError::Store(format!("{what} failed (error {code})"))
}

pub fn get(target: &str) -> Result<Option<Secret>, SecretError> {
    let target = wide(target);
    let mut credential: *mut CREDENTIALW = ptr::null_mut();
    // SAFETY: `target` is NUL-terminated and outlives the call; on success
    // `credential` points to a block Windows allocated, freed below.
    let ok = unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut credential) };
    if ok == 0 {
        // SAFETY: as in `failed`.
        if unsafe { GetLastError() } == ERROR_NOT_FOUND {
            return Ok(None);
        }
        return Err(failed("reading"));
    }
    // SAFETY: CredReadW succeeded, so `credential` is valid until CredFree,
    // and its blob holds `CredentialBlobSize` bytes (or is null).
    let mut bytes = unsafe {
        let c = &*credential;
        if c.CredentialBlob.is_null() || c.CredentialBlobSize == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec()
        }
    };
    // SAFETY: `credential` came from CredReadW and is freed once.
    unsafe { CredFree(credential as *const c_void) };
    let text = decode_blob(&bytes);
    bytes.fill(0);
    match text {
        Some(text) if !text.is_empty() => Ok(Some(Secret::new(text))),
        Some(_) => Ok(None),
        None => Err(SecretError::Store("the saved value isn't text".into())),
    }
}

pub fn set(target: &str, value: &Secret) -> Result<(), SecretError> {
    let mut target = wide(target);
    let mut user = wide(USER);
    let mut blob = value.expose().as_bytes().to_vec();
    let size = u32::try_from(blob.len()).map_err(|_| SecretError::TooLong)?;
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_mut_ptr(),
        CredentialBlobSize: size,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        UserName: user.as_mut_ptr(),
        ..Default::default()
    };
    // SAFETY: every pointer in `credential` points into a buffer above that
    // lives until after the call; Windows copies what it keeps.
    let ok = unsafe { CredWriteW(&credential, 0) };
    blob.fill(0);
    if ok == 0 {
        return Err(failed("saving"));
    }
    Ok(())
}

pub fn delete(target: &str) -> Result<(), SecretError> {
    let target = wide(target);
    // SAFETY: `target` is NUL-terminated and outlives the call.
    let ok = unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) };
    // SAFETY: as in `failed`.
    if ok == 0 && unsafe { GetLastError() } != ERROR_NOT_FOUND {
        return Err(failed("removing"));
    }
    Ok(())
}
