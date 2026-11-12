//! The speakers' volume (the default output device), through Core Audio.

use anyhow::Result;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender};
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};

use super::com;

fn endpoint() -> Result<IAudioEndpointVolume> {
    com();
    unsafe {
        let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = devices.GetDefaultAudioEndpoint(eRender, eConsole)?;
        Ok(device.Activate(CLSCTX_ALL, None)?)
    }
}

/// 0..=100
pub fn volume() -> Result<u32> {
    Ok(unsafe { (endpoint()?.GetMasterVolumeLevelScalar()? * 100.0).round() as u32 })
}

pub fn set_volume(percent: u32) -> Result<u32> {
    let ep = endpoint()?;
    unsafe {
        ep.SetMasterVolumeLevelScalar(percent.min(100) as f32 / 100.0, std::ptr::null())?;
        if percent > 0 {
            ep.SetMute(false, std::ptr::null())?;
        }
        Ok((ep.GetMasterVolumeLevelScalar()? * 100.0).round() as u32)
    }
}

pub fn mute() -> Result<bool> {
    Ok(unsafe { endpoint()?.GetMute()?.as_bool() })
}

pub fn set_mute(on: bool) -> Result<bool> {
    let ep = endpoint()?;
    unsafe {
        ep.SetMute(on, std::ptr::null())?;
        Ok(ep.GetMute()?.as_bool())
    }
}
