//! Shared atlas decoding: isolated frames with premultiplied BGRA pixels.
use serde_json::Value;
pub(crate) struct Sprite {
    pub(crate) frames: Vec<Vec<u8>>,
    pub(crate) cell_width: u32,
    pub(crate) cell_height: u32,
}
impl Sprite {
    #[cfg(any(target_os = "windows", test))]
    pub fn empty() -> Self {
        Self {
            frames: Vec::new(),
            cell_width: 0,
            cell_height: 0,
        }
    }
    pub fn load(asset: &Value) -> Option<Self> {
        let file = std::fs::File::open(asset["spritePath"].as_str()?).ok()?;
        let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().ok()?;
        let (width, height) = (reader.info().width, reader.info().height);
        let (cell_width, cell_height) = (
            asset["cellWidth"].as_u64()? as u32,
            asset["cellHeight"].as_u64()? as u32,
        );
        if cell_width == 0
            || cell_height == 0
            || u64::from(width) * u64::from(height) > 16_000_000
            || width != cell_width * 8
            || height != cell_height * asset["rows"].as_u64()? as u32
        {
            return None;
        }
        let mut pixels = vec![0; reader.output_buffer_size()?];
        let info = reader.next_frame(&mut pixels).ok()?;
        if info.color_type != png::ColorType::Rgba {
            return None;
        }
        pixels.truncate(info.buffer_size());
        for pixel in pixels.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
            // Cairo ARGB32 (little endian) and Windows both receive premultiplied BGRA.
            // Invisible white/black RGB must not tint interpolated outline pixels.
            let alpha = u16::from(pixel[3]);
            for channel in &mut pixel[..3] {
                *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
            }
        }
        // Isolate cells before resampling; the next animation frame is not an edge pixel.
        let frames = (0..8 * height / cell_height)
            .map(|index| {
                let mut frame = vec![0; (cell_width * cell_height * 4) as usize];
                for y in 0..cell_height {
                    let src = ((index / 8 * cell_height + y) * width + index % 8 * cell_width)
                        as usize
                        * 4;
                    let dst = (y * cell_width * 4) as usize;
                    frame[dst..dst + (cell_width * 4) as usize]
                        .copy_from_slice(&pixels[src..src + (cell_width * 4) as usize]);
                }
                frame
            })
            .collect();
        Some(Self {
            frames,
            cell_width,
            cell_height,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn frames_are_isolated_and_transparent_rgb_is_premultiplied() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let mut encoder = png::Encoder::new(&mut file, 8, 9);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let pixels = (0..72u8)
            .flat_map(|index| [index, 128, 255, if index == 0 { 0 } else { 128 }])
            .collect::<Vec<_>>();
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        let mut asset =
            serde_json::json!({"spritePath":file.path(),"cellWidth":1,"cellHeight":1,"rows":9});
        let sprite = Sprite::load(&asset).unwrap();
        assert_eq!((sprite.cell_width, sprite.cell_height), (1, 1));
        assert_eq!(sprite.frames.len(), 72);
        assert_eq!(sprite.frames[0], [0, 0, 0, 0]);
        assert_eq!(sprite.frames[17], [128, 64, 9, 128]);
        assert!(Sprite::empty().frames.is_empty());
        asset["cellWidth"] = 0.into();
        assert!(Sprite::load(&asset).is_none());
    }
}
