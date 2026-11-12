//! Keys and text, through SendInput: chords like "ctrl+shift+m", media keys, and typing any text.

use anyhow::{Result, bail};
use windows::Win32::UI::Input::KeyboardAndMouse::*;

/// What the app does around a chord of hers that holds one of its own system hotkeys (Esc stops her job): `true` lets
/// go of them, `false` takes them back. Windows hands a hotkey to whoever registered it, even when she sends it, so her
/// Esc for Steam stopped her own job and Steam never got it.
static OWN_KEYS: std::sync::OnceLock<Box<dyn Fn(bool) + Send + Sync>> = std::sync::OnceLock::new();

pub fn around_own_keys(f: impl Fn(bool) + Send + Sync + 'static) {
    let _ = OWN_KEYS.set(Box::new(f));
}

/// Presses a chord ("ctrl+shift+m", "enter", "media_play_pause", "alt+f4"): modifiers down, key, all up.
pub fn press(chord: &str) -> Result<()> {
    let names: Vec<String> = chord.split('+').map(|k| k.trim().to_lowercase()).filter(|k| !k.is_empty()).collect();
    if names.is_empty() {
        bail!("no keys given");
    }
    let mut vks = Vec::new();
    for n in &names {
        match vk(n) {
            Some(v) => vks.push(v),
            None => bail!("unknown key \"{n}\""),
        }
    }
    let mut inputs: Vec<INPUT> = vks.iter().map(|&v| key(v, false)).collect();
    inputs.extend(vks.iter().rev().map(|&v| key(v, true)));
    let own = vks.contains(&VK_ESCAPE).then(|| OWN_KEYS.get()).flatten();
    if let Some(f) = own {
        f(true);
    }
    let sent = send(&inputs);
    if let Some(f) = own {
        // The app reads the key after it's sent: taking the hotkey back at once caught the Esc all the same.
        std::thread::sleep(std::time::Duration::from_millis(150));
        f(false);
    }
    sent
}

/// Types text as Unicode keystrokes (any language, no clipboard involved).
pub fn type_text(text: &str) -> Result<()> {
    let mut inputs = Vec::new();
    for unit in text.encode_utf16() {
        if unit == '\n' as u16 {
            inputs.push(key(VK_RETURN, false));
            inputs.push(key(VK_RETURN, true));
            continue;
        }
        for up in [false, true] {
            inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0), wScan: unit, dwFlags: if up { KEYEVENTF_UNICODE | KEYEVENTF_KEYUP } else { KEYEVENTF_UNICODE }, time: 0, dwExtraInfo: 0 },
                },
            });
        }
    }
    send(&inputs)
}

fn send(inputs: &[INPUT]) -> Result<()> {
    let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize != inputs.len() {
        bail!("Windows blocked the keystrokes (an app running as administrator is in front?)");
    }
    Ok(())
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    // Keys like the arrows and Home/End are "extended": without the flag some apps read them as the numpad.
    let extended = matches!(vk, VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN | VK_HOME | VK_END | VK_PRIOR | VK_NEXT | VK_INSERT | VK_DELETE | VK_RMENU | VK_RCONTROL);
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    if extended {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } } }
}

fn vk(name: &str) -> Option<VIRTUAL_KEY> {
    Some(match name {
        "ctrl" | "control" => VK_CONTROL,
        "shift" => VK_SHIFT,
        "alt" => VK_MENU,
        "win" | "windows" | "meta" | "super" => VK_LWIN,
        "enter" | "return" => VK_RETURN,
        "esc" | "escape" => VK_ESCAPE,
        "tab" => VK_TAB,
        "space" | "spacebar" => VK_SPACE,
        "backspace" => VK_BACK,
        "delete" | "del" => VK_DELETE,
        "insert" | "ins" => VK_INSERT,
        "home" => VK_HOME,
        "end" => VK_END,
        "pageup" | "page_up" | "pgup" => VK_PRIOR,
        "pagedown" | "page_down" | "pgdn" => VK_NEXT,
        "up" => VK_UP,
        "down" => VK_DOWN,
        "left" => VK_LEFT,
        "right" => VK_RIGHT,
        "printscreen" | "prtsc" => VK_SNAPSHOT,
        "media_play_pause" | "play_pause" => VK_MEDIA_PLAY_PAUSE,
        "media_next" | "next_track" => VK_MEDIA_NEXT_TRACK,
        "media_previous" | "media_prev" | "previous_track" => VK_MEDIA_PREV_TRACK,
        "media_stop" => VK_MEDIA_STOP,
        "volume_up" => VK_VOLUME_UP,
        "volume_down" => VK_VOLUME_DOWN,
        "volume_mute" => VK_VOLUME_MUTE,
        _ => {
            if let Some(n) = name.strip_prefix('f').and_then(|n| n.parse::<u16>().ok()).filter(|n| (1..=24).contains(n)) {
                return Some(VIRTUAL_KEY(VK_F1.0 + n - 1));
            }
            let mut chars = name.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_alphanumeric() => VIRTUAL_KEY(c.to_ascii_uppercase() as u16),
                (Some(c), None) => {
                    // Punctuation: ask the keyboard layout which key types it.
                    let r = unsafe { VkKeyScanW(c as u16) };
                    if r == -1 {
                        return None;
                    }
                    VIRTUAL_KEY((r & 0xff) as u16)
                }
                _ => return None,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::vk;

    #[test]
    fn names() {
        assert!(vk("ctrl").is_some());
        assert!(vk("f5").is_some());
        assert!(vk("m").is_some());
        assert!(vk("media_play_pause").is_some());
        assert!(vk("nonsense").is_none());
    }
}
