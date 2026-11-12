//! Her API keys, kept encrypted by Windows (Credential Manager, "capricorn-engram:<provider>"), never in her files.

use anyhow::{Result, bail};
use windows::Win32::Foundation::FILETIME;
use windows::Win32::Security::Credentials::{CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW, CredWriteW};
use windows::core::{PCWSTR, PWSTR};

use super::wide;

fn target(provider: &str) -> Vec<u16> {
    wide(&format!("capricorn-engram:{provider}"))
}

/// The key saved for `provider`, if there is one.
pub fn key_get(provider: &str) -> Option<String> {
    let name = target(provider);
    let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
    unsafe {
        CredReadW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None, &mut cred).ok()?;
        let c = &*cred;
        let blob = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec();
        CredFree(cred as *const _);
        String::from_utf8(blob).ok().filter(|k| !k.is_empty())
    }
}

/// Saves `key` for `provider` (replacing one saved before).
pub fn key_set(provider: &str, key: &str) -> Result<()> {
    if key.len() > 2560 {
        bail!("that's too long to be a key");
    }
    let mut name = target(provider);
    let mut user = wide("capricorn-engram");
    let mut blob = key.as_bytes().to_vec();
    let cred = CREDENTIALW {
        Flags: CRED_FLAGS(0),
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(name.as_mut_ptr()),
        Comment: PWSTR::null(),
        LastWritten: FILETIME::default(),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        AttributeCount: 0,
        Attributes: std::ptr::null_mut(),
        TargetAlias: PWSTR::null(),
        UserName: PWSTR(user.as_mut_ptr()),
    };
    unsafe { CredWriteW(&cred, 0)? };
    Ok(())
}

/// Forgets the key saved for `provider`.
pub fn key_delete(provider: &str) {
    let name = target(provider);
    let _ = unsafe { CredDeleteW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None) };
}
