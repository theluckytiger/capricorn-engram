//! Her state, settings and the events everything else talks through.

pub mod answer;
pub mod bond;
pub mod journal;
pub mod notes;
pub mod reminders;
pub mod settings;

pub use settings::Settings;

/// A JSON file of hers as text, without the byte-order mark some editors put first (Windows PowerShell does): read
/// with it, her whole settings file counted as broken and she started over from her defaults.
pub fn read_json_text(path: impl AsRef<std::path::Path>) -> std::io::Result<String> {
    let text = std::fs::read_to_string(path)?;
    Ok(match text.strip_prefix('\u{feff}') {
        Some(rest) => rest.to_string(),
        None => text,
    })
}
