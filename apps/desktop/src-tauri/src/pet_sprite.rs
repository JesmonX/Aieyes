//! Alpha-composited native pet: no WebView dependency and no activation on click.
use super::*;
use windows::core::BOOL;
pub(super) struct Sprite {
    frames: Vec<Vec<u8>>,
    cell_width: u32,
    cell_height: u32,
}
impl Sprite {
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
        if u64::from(width) * u64::from(height) > 16_000_000
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
            // GDI+ and UpdateLayeredWindow both receive premultiplied BGRA.
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
pub(super) unsafe fn reduced_motion() -> bool {
    let mut enabled = BOOL(1);
    let _ = SystemParametersInfoW(
        SPI_GETCLIENTAREAANIMATION,
        0,
        Some((&mut enabled as *mut BOOL).cast()),
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
    );
    !enabled.as_bool() || high_contrast()
}
pub(super) unsafe fn paint(
    hwnd: HWND,
    sprite: &Sprite,
    state: &aieyes_core::pets::PetState,
    frame: u32,
    font: *mut GpFont,
    info: &Value,
) {
    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect).is_err() {
        return;
    }
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    let dc = GetDC(None);
    let memory = CreateCompatibleDC(Some(dc));
    let header = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = null_mut();
    let Ok(bitmap) = CreateDIBSection(Some(dc), &header, DIB_RGB_COLORS, &mut bits, None, 0) else {
        let _ = DeleteDC(memory);
        ReleaseDC(None, dc);
        return;
    };
    std::ptr::write_bytes(bits, 0, (width * height * 4) as usize);
    let old = SelectObject(memory, bitmap.into());
    draw(
        bits.cast(),
        width,
        height,
        sprite,
        frame,
        font,
        &state.label,
        info,
    );
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
        BlendFlags: 0,
    };
    let _ = UpdateLayeredWindow(
        hwnd,
        Some(dc),
        Some(&POINT {
            x: rect.left,
            y: rect.top,
        }),
        Some(&SIZE {
            cx: width,
            cy: height,
        }),
        Some(memory),
        Some(&POINT { x: 0, y: 0 }),
        COLORREF(0),
        Some(&blend),
        ULW_ALPHA,
    );
    SelectObject(memory, old);
    let _ = DeleteObject(bitmap.into());
    let _ = DeleteDC(memory);
    ReleaseDC(None, dc);
}
// Render into premultiplied BGRA memory: HDC rendering loses the alpha channel.
#[allow(clippy::too_many_arguments)]
unsafe fn draw(
    bits: *mut u8,
    width: i32,
    height: i32,
    sprite: &Sprite,
    frame: u32,
    font: *mut GpFont,
    label: &str,
    info: &Value,
) {
    let scale = width as f32 / 144.0;
    let mut graphics = null_mut();
    let mut canvas = null_mut();
    if GdipCreateBitmapFromScan0(
        width,
        height,
        width * 4,
        0xe200b,
        Some(bits.cast()),
        &mut canvas,
    ) == GDI_OK
        && GdipGetImageGraphicsContext(canvas.cast(), &mut graphics) == GDI_OK
    {
        GdipScaleWorldTransform(graphics, scale, scale, MatrixOrderPrepend);
        GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
        GdipSetInterpolationMode(graphics, InterpolationModeHighQualityBicubic);
        GdipSetPixelOffsetMode(graphics, PixelOffsetModeHalf);
        let mut image = null_mut();
        if let Some(pixels) = sprite.frames.get(frame as usize)
            && GdipCreateBitmapFromScan0(
                sprite.cell_width as i32,
                sprite.cell_height as i32,
                (sprite.cell_width * 4) as i32,
                0xe200b,
                Some(pixels.as_ptr()),
                &mut image,
            ) == GDI_OK
        {
            GdipDrawImageRectRect(
                graphics,
                image.cast(),
                6.0,
                0.0,
                132.0,
                143.0,
                0.0,
                0.0,
                sprite.cell_width as f32,
                sprite.cell_height as f32,
                UnitPixel,
                std::ptr::null(),
                0,
                null_mut(),
            );
            GdipDisposeImage(image.cast());
        }
        let mut brush = null_mut();
        GdipCreateSolidFill(
            if high_contrast() {
                system_argb(COLOR_WINDOW)
            } else {
                0xeef6f6f8
            },
            &mut brush,
        );
        let (x, y, w, h) = (3.0, 145.0, 138.0, height as f32 / scale - 147.0);
        let diameter = 20.0_f32.min(h);
        let mut path = null_mut();
        if GdipCreatePath(FillModeWinding, &mut path) == GDI_OK {
            for (left, top, angle) in [
                (x, y, 180.0),
                (x + w - diameter, y, 270.0),
                (x + w - diameter, y + h - diameter, 0.0),
                (x, y + h - diameter, 90.0),
            ] {
                GdipAddPathArc(path, left, top, diameter, diameter, angle, 90.0);
            }
            GdipClosePathFigure(path);
            GdipFillPath(graphics, brush.cast(), path);
            GdipDeletePath(path);
        }
        GdipDeleteBrush(brush.cast());
        let label = if sprite.frames.is_empty() {
            "请导入桌宠"
        } else if info["recovery"] == "failed" {
            "恢复失败"
        } else {
            label
        };
        let text: Vec<u16> = label.encode_utf16().collect();
        let mut format = null_mut();
        GdipCreateStringFormat(0, 0, &mut format);
        GdipSetStringFormatAlign(format, StringAlignmentCenter);
        GdipSetStringFormatLineAlign(format, StringAlignmentCenter);
        GdipSetTextRenderingHint(graphics, TextRenderingHintAntiAliasGridFit);
        GdipCreateSolidFill(
            if high_contrast() {
                system_argb(COLOR_WINDOWTEXT)
            } else {
                0xff243049
            },
            &mut brush,
        );
        GdipDrawString(
            graphics,
            PCWSTR(text.as_ptr()),
            text.len() as i32,
            font,
            &RectF {
                X: 3.0,
                Y: 143.0,
                Width: 138.0,
                Height: height as f32 / scale - 143.0,
            },
            format,
            brush.cast(),
        );
        GdipDeleteBrush(brush.cast());
        GdipDeleteStringFormat(format);
        GdipDeleteGraphics(graphics);
    }
    if !canvas.is_null() {
        GdipDisposeImage(canvas.cast());
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn imported_sprite() -> Sprite {
        let root = tempfile::tempdir().unwrap();
        let store = aieyes_core::store::Store::open(root.path()).unwrap();
        let value = aieyes_core::pet_import::current(&store, false).unwrap();
        Sprite::load(&value["pet"]).unwrap()
    }
    #[test]
    fn imported_sheet_has_complete_transparent_rgba_frames() {
        let sprite = imported_sprite();
        assert_eq!(sprite.frames.len(), 72);
        assert_eq!(
            sprite.frames.iter().map(Vec::len).sum::<usize>(),
            1536 * 1872 * 4
        );
        assert_eq!(sprite.frames[0][3], 0);
        assert!(
            sprite.frames[0]
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[3] == 255)
        );
        assert!(
            sprite
                .frames
                .iter()
                .flat_map(|frame| frame.as_chunks::<4>().0)
                .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3])
        );
    }
    #[test]
    fn transparent_rgb_is_removed_before_frame_interpolation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("transparent.png");
        let mut encoder = png::Encoder::new(std::fs::File::create(&path).unwrap(), 8, 9);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut pixels = vec![255; 8 * 9 * 4];
        pixels[..8].copy_from_slice(&[255, 255, 255, 0, 100, 50, 200, 128]);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        let sprite = Sprite::load(
            &serde_json::json!({"spritePath":path,"cellWidth":1,"cellHeight":1,"rows":9}),
        )
        .unwrap();
        assert_eq!(sprite.frames[0], [0, 0, 0, 0]);
        assert_eq!(sprite.frames[1], [100, 25, 50, 128]);
    }
    #[test]
    fn native_render_preserves_alpha_at_multiple_dpi() {
        unsafe {
            let mut token = 0;
            let input = GdiplusStartupInput {
                GdiplusVersion: 1,
                ..Default::default()
            };
            assert_eq!(GdiplusStartup(&mut token, &input, null_mut()), GDI_OK);
            let sprite = imported_sprite();
            for scale in [0.5_f64, 1.0, 1.25, 2.0, 3.0] {
                let (width, height) = (
                    (144.0 * scale).round() as i32,
                    (168.0 * scale).round() as i32,
                );
                for row in 0..9 {
                    let mut pixels = vec![0u8; (width * height * 4) as usize];
                    draw(
                        pixels.as_mut_ptr(),
                        width,
                        height,
                        &sprite,
                        row * 8,
                        null_mut(),
                        "",
                        &serde_json::json!({}),
                    );
                    assert_eq!(pixels[3], 0, "transparent corner");
                    assert!(
                        pixels[..(f64::from(width) * 140.0 * scale * 4.0) as usize]
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .any(|p| p[3] > 0),
                        "pet frame is visible: row {row}"
                    );
                    assert!(
                        pixels
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]),
                        "premultiplied alpha"
                    );
                }
            }
            GdiplusShutdown(token);
        }
    }
}
