//! The apps she can open: everything in the Start menu's "All apps" (desktop programs, Store apps like Spotify,
//! games with shortcuts), read from the shell's AppsFolder, the same list `Get-StartApps` shows.

use anyhow::{Result, bail};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{BHID_EnumItems, IEnumShellItems, IShellItem, SHCreateItemFromParsingName, SIGDN, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING};
use windows::core::w;

use super::com;
use crate::platform::AppEntry;

pub fn apps() -> Result<Vec<AppEntry>> {
    com();
    let mut out = Vec::new();
    unsafe {
        let folder: IShellItem = SHCreateItemFromParsingName(w!("shell:AppsFolder"), None)?;
        let items: IEnumShellItems = folder.BindToHandler(None, &BHID_EnumItems)?;
        loop {
            let mut batch = [None];
            let mut fetched = 0u32;
            if items.Next(&mut batch, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            let Some(item) = batch[0].take() else { break };
            let (Some(name), Some(id)) = (display(&item, SIGDN_NORMALDISPLAY), display(&item, SIGDN_PARENTRELATIVEPARSING)) else { continue };
            out.push(AppEntry { name, id });
        }
    }
    Ok(out)
}

unsafe fn display(item: &IShellItem, kind: SIGDN) -> Option<String> {
    unsafe {
        let p = item.GetDisplayName(kind).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

/// Starts an app by its AppsFolder id, the way the Start menu does.
pub fn launch(id: &str) -> Result<()> {
    let status = std::process::Command::new("explorer.exe").arg(format!("shell:AppsFolder\\{id}")).spawn();
    match status {
        Ok(_) => Ok(()),
        Err(e) => bail!("couldn't start it: {e}"),
    }
}
