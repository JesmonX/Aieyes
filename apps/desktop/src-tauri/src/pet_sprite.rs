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
// Reuse the layered-window DIB across frames; only size/DPI changes allocate a surface.
struct Dib {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    width: i32,
    height: i32,
}
impl Dib {
    unsafe fn new(width: i32, height: i32) -> Option<Self> {
        if width <= 0 || height <= 0 {
            return None;
        }
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
        let bitmap = CreateDIBSection(Some(dc), &header, DIB_RGB_COLORS, &mut bits, None, 0);
        ReleaseDC(None, dc);
        let Ok(bitmap) = bitmap else {
            let _ = DeleteDC(memory);
            return None;
        };
        let old = SelectObject(memory, bitmap.into());
        Some(Self {
            dc: memory,
            bitmap,
            old,
            bits: bits.cast(),
            width,
            height,
        })
    }
}
impl Drop for Dib {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}
#[derive(Default)]
pub(super) struct Renderer {
    surface: Option<Dib>,
    label: LabelCache,
}
#[derive(Default)]
struct LabelCache {
    key: Option<(i32, i32, String, u32, u32)>,
    pixels: Vec<u8>,
}
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn paint(
    hwnd: HWND,
    sprite: &Sprite,
    state: &aieyes_core::pets::PetState,
    frame: u32,
    font: *mut GpFont,
    info: &Value,
    renderer: &mut Renderer,
) {
    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect).is_err() {
        return;
    }
    let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
    if renderer
        .surface
        .as_ref()
        .is_none_or(|s| s.width != width || s.height != height)
    {
        renderer.surface = Dib::new(width, height);
    }
    let Some(surface) = &renderer.surface else {
        return;
    };
    std::ptr::write_bytes(surface.bits, 0, (width * height * 4) as usize);
    draw_cached(
        surface.bits,
        width,
        height,
        sprite,
        frame,
        font,
        &state.label,
        info,
        &mut renderer.label,
    );
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
        BlendFlags: 0,
    };
    let _ = UpdateLayeredWindow(
        hwnd,
        None,
        Some(&POINT {
            x: rect.left,
            y: rect.top,
        }),
        Some(&SIZE {
            cx: width,
            cy: height,
        }),
        Some(surface.dc),
        Some(&POINT { x: 0, y: 0 }),
        COLORREF(0),
        Some(&blend),
        ULW_ALPHA,
    );
}
// Render into premultiplied BGRA memory: HDC rendering loses the alpha channel.
#[allow(clippy::too_many_arguments)]
unsafe fn draw_cached(
    bits: *mut u8,
    width: i32,
    height: i32,
    sprite: &Sprite,
    frame: u32,
    font: *mut GpFont,
    label: &str,
    info: &Value,
    label_cache: &mut LabelCache,
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
        GdipDeleteGraphics(graphics);
    }
    if !canvas.is_null() {
        GdipDisposeImage(canvas.cast());
    }
    let label = if sprite.frames.is_empty() {
        "请导入桌宠"
    } else if info["recovery"] == "failed" {
        "恢复失败"
    } else if info["recovery"] == "refreshing" {
        "恢复中"
    } else {
        label
    };
    let contrast = high_contrast();
    let background = if contrast {
        system_argb(COLOR_WINDOW)
    } else {
        0xeef6f6f8
    };
    let foreground = if contrast {
        system_argb(COLOR_WINDOWTEXT)
    } else {
        0xff243049
    };
    let key = (width, height, label.to_owned(), background, foreground);
    if label_cache.key.as_ref() != Some(&key) {
        label_cache.pixels = render_label(width, height, font, label, background, foreground);
        label_cache.key = Some(key);
    }
    // Both layers contain premultiplied BGRA; source-over preserves transparent edges.
    let pixels = std::slice::from_raw_parts_mut(bits, (width * height * 4) as usize);
    for (dst, src) in pixels
        .chunks_exact_mut(4)
        .zip(label_cache.pixels.chunks_exact(4))
    {
        let inverse = 255 - u16::from(src[3]);
        for channel in 0..4 {
            dst[channel] = (u16::from(src[channel])
                + (u16::from(dst[channel]) * inverse + 127) / 255)
                .min(255) as u8;
        }
    }
}
// Grayscale coverage at 3x physical resolution, then area-filter once. ClearType
// subpixels cannot be composed correctly onto a transparent layered window.
unsafe fn render_label(
    width: i32,
    height: i32,
    font: *mut GpFont,
    label: &str,
    background: u32,
    foreground: u32,
) -> Vec<u8> {
    let high = vec![0u8; (width * height * 4 * 9) as usize];
    let mut canvas = null_mut();
    let mut graphics = null_mut();
    let scale = width as f32 / 144.0;
    if GdipCreateBitmapFromScan0(
        width * 3,
        height * 3,
        width * 12,
        0xe200b,
        Some(high.as_ptr()),
        &mut canvas,
    ) == GDI_OK
        && GdipGetImageGraphicsContext(canvas.cast(), &mut graphics) == GDI_OK
    {
        GdipScaleWorldTransform(graphics, scale * 3.0, scale * 3.0, MatrixOrderPrepend);
        GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
        let text: Vec<u16> = label.encode_utf16().collect();
        let mut format = null_mut();
        GdipCreateStringFormat(0, 0, &mut format);
        GdipSetStringFormatAlign(format, StringAlignmentCenter);
        GdipSetStringFormatLineAlign(format, StringAlignmentCenter);
        let mut measured = RectF::default();
        let layout = RectF {
            X: 0.0,
            Y: 0.0,
            Width: 120.0,
            Height: height as f32 / scale - 143.0,
        };
        let measured_ok = GdipMeasureString(
            graphics,
            PCWSTR(text.as_ptr()),
            text.len() as i32,
            font,
            &layout,
            format,
            &mut measured,
            null_mut(),
            null_mut(),
        ) == GDI_OK;
        let w = if measured_ok {
            (measured.Width.ceil() + 18.0).clamp(30.0, 138.0)
        } else {
            138.0
        };
        let (x, y, h) = ((144.0 - w) / 2.0, 145.0, height as f32 / scale - 147.0);
        let mut brush = null_mut();
        GdipCreateSolidFill(background, &mut brush);
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
        GdipSetTextRenderingHint(graphics, TextRenderingHintAntiAliasGridFit);
        GdipCreateSolidFill(foreground, &mut brush);
        GdipDrawString(
            graphics,
            PCWSTR(text.as_ptr()),
            text.len() as i32,
            font,
            &RectF {
                X: x + 9.0,
                Y: 143.0,
                Width: w - 18.0,
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
    downsample_label(&high, width as usize, height as usize)
}
fn downsample_label(high: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut pixels = vec![0; width * height * 4];
    for y in 0..height {
        for x in 0..width {
            for channel in 0..4 {
                let mut sum = 0u16;
                for dy in 0..3 {
                    for dx in 0..3 {
                        sum +=
                            u16::from(high[((y * 3 + dy) * width * 3 + x * 3 + dx) * 4 + channel]);
                    }
                }
                pixels[(y * width + x) * 4 + channel] = ((sum + 4) / 9) as u8;
            }
        }
    }
    pixels
}
#[cfg(test)]
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
    draw_cached(
        bits,
        width,
        height,
        sprite,
        frame,
        font,
        label,
        info,
        &mut LabelCache::default(),
    );
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
    fn status_text_has_coverage_and_invalidates_at_physical_size() {
        unsafe {
            let mut token = 0;
            assert_eq!(
                GdiplusStartup(
                    &mut token,
                    &GdiplusStartupInput {
                        GdiplusVersion: 1,
                        ..Default::default()
                    },
                    null_mut()
                ),
                GDI_OK
            );
            {
                let font = PetFont::new();
                assert!(!font.font.is_null());
                for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
                    let width = (144.0_f64 * scale).round() as i32;
                    let height = (186.0_f64 * scale).round() as i32;
                    let pixels = render_label(
                        width,
                        height,
                        font.font,
                        "等待1 · 工作2\n思考1",
                        0xeef6f6f8,
                        0xff243049,
                    );
                    let short =
                        render_label(width, height, font.font, "空闲", 0xeef6f6f8, 0xff243049);
                    let extent = |buffer: &[u8]| {
                        let columns: Vec<_> = buffer
                            .chunks_exact(4)
                            .enumerate()
                            .filter_map(|(index, p)| (p[3] > 0).then_some(index % width as usize))
                            .collect();
                        (
                            *columns.iter().min().unwrap(),
                            *columns.iter().max().unwrap(),
                        )
                    };
                    let (left, right) = extent(&short);
                    let (long_left, long_right) = extent(&pixels);
                    assert!(
                        right - left < long_right - long_left,
                        "background follows label width"
                    );
                    assert!(
                        (left as i32 + right as i32 - width + 1).abs() <= 2,
                        "label remains centered at every DPI"
                    );
                    assert_eq!(pixels.len(), (width * height * 4) as usize);
                    assert_eq!(pixels[3], 0);
                    assert!(
                        pixels.chunks_exact(4).any(|p| p[3] > 240 && p[0] < 180),
                        "text strokes are visible"
                    );
                    assert!(
                        pixels
                            .chunks_exact(4)
                            .all(|p| p[..3].iter().all(|c| *c <= p[3])),
                        "filtered coverage stays premultiplied"
                    );
                }
                let sprite = imported_sprite();
                let mut cache = LabelCache::default();
                let mut pixels = vec![0; 144 * 168 * 4];
                draw_cached(
                    pixels.as_mut_ptr(),
                    144,
                    168,
                    &sprite,
                    0,
                    font.font,
                    "空闲",
                    &serde_json::json!({}),
                    &mut cache,
                );
                let first = cache.pixels.clone();
                let allocation = cache.pixels.as_ptr();
                draw_cached(
                    pixels.as_mut_ptr(),
                    144,
                    168,
                    &sprite,
                    1,
                    font.font,
                    "空闲",
                    &serde_json::json!({}),
                    &mut cache,
                );
                assert_eq!(
                    cache.pixels.as_ptr(),
                    allocation,
                    "sprite frames reuse text pixels"
                );
                assert_eq!(cache.pixels, first);
                draw_cached(
                    pixels.as_mut_ptr(),
                    144,
                    168,
                    &sprite,
                    1,
                    font.font,
                    "等待输入",
                    &serde_json::json!({}),
                    &mut cache,
                );
                assert_ne!(cache.pixels, first);
                let mut larger = vec![0; 288 * 336 * 4];
                draw_cached(
                    larger.as_mut_ptr(),
                    288,
                    336,
                    &sprite,
                    1,
                    font.font,
                    "等待输入",
                    &serde_json::json!({}),
                    &mut cache,
                );
                assert_eq!(cache.pixels.len(), larger.len());
            }
            GdiplusShutdown(token);
        }
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
