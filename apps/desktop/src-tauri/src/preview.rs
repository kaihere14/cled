//! Small PNG thumbnails of clipboard images for the UI.
//!
//! The UI never receives full-size pixels: a 4K screenshot is ~33 MB of RGBA, a thumbnail
//! is typically well under 100 KB.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use cled_clipboard::Image;
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageBuffer, ImageEncoder, Rgba, imageops};

/// Longest edge of a thumbnail, in pixels. Twice the display size for high-DPI screens.
const MAX_EDGE: u32 = 480;

/// Encodes a thumbnail of `image` as a `data:image/png;base64,...` URL.
pub fn data_url(image: &Image) -> Option<String> {
    let source =
        ImageBuffer::<Rgba<u8>, &[u8]>::from_raw(image.width(), image.height(), image.rgba())?;
    let (width, height) = thumbnail_size(image.width(), image.height());
    let thumbnail = imageops::thumbnail(&source, width, height);

    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(&thumbnail, width, height, ExtendedColorType::Rgba8)
        .ok()?;
    Some(format!("data:image/png;base64,{}", STANDARD.encode(png)))
}

/// Scales `width`×`height` down so the longest edge is at most `MAX_EDGE`, keeping the aspect
/// ratio. Never scales up, and never returns a zero dimension.
fn thumbnail_size(width: u32, height: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= MAX_EDGE {
        return (width.max(1), height.max(1));
    }
    let scale =
        |edge: u32| ((u64::from(edge) * u64::from(MAX_EDGE)) / u64::from(longest)).max(1) as u32;
    (scale(width), scale(height))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_images_keep_their_size() {
        assert_eq!(thumbnail_size(100, 50), (100, 50));
        assert_eq!(thumbnail_size(480, 480), (480, 480));
    }

    #[test]
    fn large_images_scale_by_longest_edge() {
        assert_eq!(thumbnail_size(2560, 1440), (480, 270));
        assert_eq!(thumbnail_size(1000, 4000), (120, 480));
    }

    #[test]
    fn extreme_aspect_ratios_never_hit_zero() {
        assert_eq!(thumbnail_size(10_000, 1), (480, 1));
    }

    #[test]
    fn encodes_a_png_data_url() {
        let image = Image::from_rgba(2, 2, vec![255; 16]).unwrap();
        let url = data_url(&image).unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
    }
}
