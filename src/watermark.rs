use std::fs;
use std::io::BufWriter;
use std::path::Path;

use ab_glyph::{Font, FontRef, GlyphId, PxScale, ScaleFont};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::{self, FilterType};
use image::{DynamicImage, ImageEncoder, Rgba, RgbaImage};

use crate::config::{Watermark, IMAGE_QUALITY};
use crate::logger::Logger;

// ── Font resolution ──────────────────────────────────────────────────────────

/// Attempt to locate and load a font file by name.
/// Search order: absolute → relative to `base_path` → Windows Fonts directory.
fn load_font_data(name: &str, base_path: &Path) -> Option<Vec<u8>> {
    let p = Path::new(name);
    if p.is_absolute() && p.exists() {
        return fs::read(p).ok();
    }
    let rel = base_path.join(name);
    if rel.exists() {
        return fs::read(rel).ok();
    }
    if let Ok(windir) = std::env::var("WINDIR") {
        let sys = Path::new(&windir).join("Fonts").join(name);
        if sys.exists() {
            return fs::read(sys).ok();
        }
    }
    None
}

// ── Text measurement & drawing ───────────────────────────────────────────────

fn measure_text(font: &FontRef<'_>, scale: PxScale, text: &str) -> (f32, f32) {
    let scaled = font.as_scaled(scale);
    let mut max_width: f32 = 0.0;
    let line_count = text.lines().count().max(1) as f32;

    for line in text.lines() {
        let mut w: f32 = 0.0;
        let mut prev: Option<GlyphId> = None;
        for ch in line.chars() {
            let gid = scaled.glyph_id(ch);
            if let Some(p) = prev {
                w += scaled.kern(p, gid);
            }
            w += scaled.h_advance(gid);
            prev = Some(gid);
        }
        max_width = max_width.max(w);
    }

    let height = scaled.height() * line_count
        + scaled.line_gap() * (line_count - 1.0).max(0.0);
    (max_width, height)
}

/// Alpha-blend a single channel value.
#[inline(always)]
fn blend(fg: u8, bg: u8, a: f32) -> u8 {
    (fg as f32 * a + bg as f32 * (1.0 - a)).min(255.0) as u8
}

/// Rasterise text onto `image` using `ab_glyph` outlines.
fn draw_text(
    image: &mut RgbaImage,
    font: &FontRef<'_>,
    scale: PxScale,
    x: f32,
    y: f32,
    text: &str,
    color: [u8; 4],
) {
    let scaled = font.as_scaled(scale);
    let (img_w, img_h) = (image.width(), image.height());

    for (line_idx, line) in text.lines().enumerate() {
        let mut cx = x;
        let baseline = y + scaled.ascent() + line_idx as f32 * (scaled.height() + scaled.line_gap());
        let mut prev: Option<GlyphId> = None;

        for ch in line.chars() {
            let gid = scaled.glyph_id(ch);
            if let Some(p) = prev {
                cx += scaled.kern(p, gid);
            }

            let glyph = gid.with_scale_and_position(scale, ab_glyph::point(cx, baseline));
            if let Some(outlined) = font.outline_glyph(glyph) {
                let bb = outlined.px_bounds();
                outlined.draw(|gx, gy, cov| {
                    let px = gx as i64 + bb.min.x.floor() as i64;
                    let py = gy as i64 + bb.min.y.floor() as i64;
                    if px >= 0 && py >= 0 && (px as u32) < img_w && (py as u32) < img_h {
                        let alpha = cov * (color[3] as f32 / 255.0);
                        if alpha > 0.004 {
                            let pixel = image.get_pixel_mut(px as u32, py as u32);
                            pixel[0] = blend(color[0], pixel[0], alpha);
                            pixel[1] = blend(color[1], pixel[1], alpha);
                            pixel[2] = blend(color[2], pixel[2], alpha);
                            pixel[3] = ((alpha * 255.0) + pixel[3] as f32 * (1.0 - alpha)).min(255.0) as u8;
                        }
                    }
                });
            }

            cx += scaled.h_advance(gid);
            prev = Some(gid);
        }
    }
}

/// Render styled text onto the RGBA canvas.
#[allow(clippy::too_many_arguments)]
fn draw_styled_text(
    image: &mut RgbaImage,
    font: &FontRef<'_>,
    scale: PxScale,
    x: f32,
    y: f32,
    text: &str,
    color: [u8; 4],
    weight: &str,
) {
    match weight {
        "bold" => {
            for offset in -1..=1 {
                draw_text(image, font, scale, x + offset as f32, y, text, color);
                draw_text(image, font, scale, x, y + offset as f32, text, color);
            }
        }
        "thin" => {
            let thin = [color[0], color[1], color[2], (color[3] as f32 * 0.7) as u8];
            draw_text(image, font, scale, x, y, text, thin);
        }
        _ => draw_text(image, font, scale, x, y, text, color),
    }
}

// ── Watermark canvas ─────────────────────────────────────────────────────────

/// Working context for watermark operations, avoiding excessive function parameters.
struct Canvas<'a> {
    rgba: &'a mut RgbaImage,
    base_path: &'a Path,
    logger: &'a mut Logger,
}

impl Canvas<'_> {
    fn width(&self) -> u32 {
        self.rgba.width()
    }

    fn height(&self) -> u32 {
        self.rgba.height()
    }

    fn apply_image_wm(&mut self, path: &str, pos_x: f64, pos_y: f64, opacity: u8, index: usize) {
        let wm_path = if Path::new(path).is_absolute() {
            Path::new(path).to_path_buf()
        } else {
            self.base_path.join(path)
        };

        let wm_img = match image::open(&wm_path) {
            Ok(i) => i,
            Err(e) => {
                self.logger.log(&format!("Watermark {} file error: {e}", index + 1));
                return;
            }
        };

        let (w, h) = (self.width(), self.height());
        let mut wm_rgba = imageops::resize(&wm_img.to_rgba8(), w / 5, h / 5, FilterType::Lanczos3);

        let factor = opacity as f32 / 100.0;
        for Rgba(px) in wm_rgba.pixels_mut() {
            px[3] = (px[3] as f32 * factor) as u8;
        }

        imageops::overlay(self.rgba, &wm_rgba, (w as f64 / pos_x) as i64, (h as f64 / pos_y) as i64);
        self.logger.log(&format!("Watermark {} added at ({}, {}) opacity {}%", index + 1, pos_x, pos_y, opacity));
    }

    /// Apply a single watermark to the canvas.
    fn apply(&mut self, wm: &Watermark, index: usize) {
        match wm {
            Watermark::Image { path, pos_x, pos_y, opacity } => {
                self.apply_image_wm(path, *pos_x, *pos_y, *opacity, index);
            }
            Watermark::Text {
                content, pos_x, pos_y, opacity,
                font_type, font_size, font_color, font_weight,
            } => {
                let data = match load_font_data(font_type, self.base_path) {
                    Some(d) => d,
                    None => {
                        self.logger.log(&format!("Watermark {}: Font {font_type} not found", index + 1));
                        return;
                    }
                };
                let font = match FontRef::try_from_slice(&data) {
                    Ok(f) => f,
                    Err(e) => {
                        self.logger.log(&format!("Watermark {}: Failed to load font: {e}", index + 1));
                        return;
                    }
                };

                let (w, h) = (self.width() as f32, self.height() as f32);
                let scale = PxScale::from(*font_size as f32);
                let (tw, th) = measure_text(&font, scale, content);
                let x = (w - tw) / *pos_x as f32;
                let y = (h - th) / *pos_y as f32;

                let factor = *opacity as f32 / 100.0;
                let color = [font_color[0], font_color[1], font_color[2], (font_color[3] as f32 * factor) as u8];

                draw_styled_text(self.rgba, &font, scale, x, y, content, color, font_weight);
                self.logger.log(&format!("Text watermark {} added at ({}, {}) opacity {}%", index + 1, pos_x, pos_y, opacity));
            }
        }
    }
}

// ── Public entry point ───────────────────────────────────────────────────────

/// Apply all configured watermarks (copyright + user-defined) to the image file.
pub fn add_watermarks(
    image_path: &Path,
    watermarks: &[Watermark],
    base_path: &Path,
    logger: &mut Logger,
) {
    let img = match image::open(image_path) {
        Ok(i) => i,
        Err(e) => {
            logger.log(&format!("Failed to open image for watermark: {e}"));
            return;
        }
    };

    let mut rgba = img.to_rgba8();

    // ── Built-in copyright watermark ─────────────────────────────────────
    if let Some(data) = load_font_data("BRADHITC.TTF", base_path) {
        if let Ok(font) = FontRef::try_from_slice(&data) {
            let scale = PxScale::from(62.0);
            let text = "   Auto Change Wallpaper By LtqX\n\nPictures all from and belong to Bing";
            let (tw, th) = measure_text(&font, scale, text);
            let x = (rgba.width() as f32 - tw) / 2.0;
            let y = (rgba.height() as f32 - th) / 1.2;
            draw_styled_text(&mut rgba, &font, scale, x, y, text, [128, 128, 128, 204], "bold");
        }
    } else {
        logger.log("Copyright font BRADHITC.TTF not found, skipping copyright watermark");
    }

    // ── User-defined watermarks ──────────────────────────────────────────
    {
        let mut canvas = Canvas { rgba: &mut rgba, base_path, logger };
        for (i, wm) in watermarks.iter().enumerate() {
            canvas.apply(wm, i);
        }
    }

    // ── Save as JPEG with quality setting ────────────────────────────────
    let rgb = DynamicImage::ImageRgba8(rgba).to_rgb8();
    let save_result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let file = fs::File::create(image_path)?;
        let encoder = JpegEncoder::new_with_quality(BufWriter::new(file), IMAGE_QUALITY);
        encoder.write_image(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8)?;
        Ok(())
    })();

    if let Err(e) = save_result {
        logger.log(&format!("Failed to save watermarked image: {e}"));
    }
}
