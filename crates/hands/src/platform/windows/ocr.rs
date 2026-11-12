//! Reading the words on a window's picture with Windows' own text reader (OCR: on this PC, quick, exact, no model).
//! For what a window draws itself (games, canvases, video titles, Electron apps), where its controls have no names.

use anyhow::{Context, Result};
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;

use super::screen::Shot;

/// Words that belong together on the screen (a button, a title, a run of text) and where they are, in screen pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    pub text: String,
    /// left, top, right, bottom.
    pub rect: [i32; 4],
}

impl Label {
    pub fn center(&self) -> (i32, i32) {
        ((self.rect[0] + self.rect[2]) / 2, (self.rect[1] + self.rect[3]) / 2)
    }
}

/// A gap this many letter heights wide between two words on a line starts another label: the reader calls a whole
/// row of a menu or a toolbar one line, and each button needs its own place.
const GAP: f32 = 1.3;

/// The labels on a picture of a window, top to bottom and left to right.
pub fn read(shot: &Shot) -> Result<Vec<Label>> {
    let engine = OcrEngine::TryCreateFromUserProfileLanguages().context("Windows has no text reader for this PC's languages")?;
    let max = OcrEngine::MaxImageDimension().unwrap_or(4096).max(1);
    let (w, h) = (shot.width, shot.height);
    if w == 0 || h == 0 {
        return Ok(Vec::new());
    }
    // Small windows read better at twice the size; big ones are shrunk to what the reader takes.
    let side = w.max(h);
    let scale = if side * 2 <= max && side <= 1600 { 2.0 } else if side > max { max as f32 / side as f32 } else { 1.0 };
    let (sw, sh) = (((w as f32 * scale) as u32).max(1), ((h as f32 * scale) as u32).max(1));
    let mut bgra = vec![255u8; (sw * sh * 4) as usize];
    for y in 0..sh {
        let from_y = ((y as f32 / scale) as u32).min(h - 1);
        for x in 0..sw {
            let from_x = ((x as f32 / scale) as u32).min(w - 1);
            let s = ((from_y * w + from_x) * 3) as usize;
            let d = ((y * sw + x) * 4) as usize;
            bgra[d] = shot.rgb[s + 2];
            bgra[d + 1] = shot.rgb[s + 1];
            bgra[d + 2] = shot.rgb[s];
        }
    }
    let writer = DataWriter::new()?;
    writer.WriteBytes(&bgra)?;
    let buffer = writer.DetachBuffer()?;
    let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(&buffer, BitmapPixelFormat::Bgra8, sw as i32, sh as i32, BitmapAlphaMode::Premultiplied)?;
    let result = engine.RecognizeAsync(&bitmap)?.join()?;
    let mut labels = Vec::new();
    for line in result.Lines()? {
        let mut words: Vec<(String, [f32; 4])> = Vec::new();
        for word in line.Words()? {
            let r = word.BoundingRect()?;
            words.push((word.Text()?.to_string(), [r.X, r.Y, r.X + r.Width, r.Y + r.Height]));
        }
        words.sort_by(|a, b| a.1[0].total_cmp(&b.1[0]));
        let mut heights: Vec<f32> = words.iter().map(|(_, r)| r[3] - r[1]).collect();
        heights.sort_by(f32::total_cmp);
        let tall = heights.get(heights.len() / 2).copied().unwrap_or(10.0).max(4.0);
        let mut group: Vec<&(String, [f32; 4])> = Vec::new();
        let mut flush = |group: &mut Vec<&(String, [f32; 4])>| {
            if group.is_empty() {
                return;
            }
            let text = group.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>().join(" ");
            let r = group.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |a, (_, r)| [a[0].min(r[0]), a[1].min(r[1]), a[2].max(r[2]), a[3].max(r[3])]);
            let at = |v: f32| (v / scale).round() as i32;
            labels.push(Label { text, rect: [shot.left + at(r[0]), shot.top + at(r[1]), shot.left + at(r[2]), shot.top + at(r[3])] });
            group.clear();
        };
        for word in &words {
            if let Some(last) = group.last()
                && word.1[0] - last.1[2] > GAP * tall
            {
                flush(&mut group);
            }
            group.push(word);
        }
        flush(&mut group);
    }
    labels.sort_by_key(|l| (l.rect[1] / 8, l.rect[0]));
    Ok(labels)
}
