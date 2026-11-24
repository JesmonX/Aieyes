//! Alpha-composited native pet: no WebView dependency and no activation on click.
use super::*;
pub(super) use crate::pet_frames::Sprite;
use windows::core::BOOL;
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
/// What the status layer shows; an equal spec at the same size reuses the cached pixels.
#[derive(Clone, PartialEq)]
struct LabelSpec {
    text: String,
    background: u32,
    foreground: u32,
    /// Phase colour of the dot before the text.
    dot: u32,
    /// Active-session count at the top right, when enabled: (text, fill, ink).
    badge: Option<(String, u32, u32)>,
    has_orbit: bool,
}
#[derive(Default)]
struct LabelCache {
    key: Option<(i32, i32, LabelSpec)>,
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
    orbit_agents: &[aieyes_core::pets::PetOrbitAgent],
    orbit_hover: Option<usize>,
    orbit_elapsed_ms: u64,
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
        orbit_agents,
        orbit_hover,
        orbit_elapsed_ms,
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
    orbit_agents: &[aieyes_core::pets::PetOrbitAgent],
    orbit_hover: Option<usize>,
    orbit_elapsed_ms: u64,
    label_cache: &mut LabelCache,
) {
    let has_orbit = !orbit_agents.is_empty();
    let (pad_x, pad_top) = aieyes_core::pets::orbit_padding(has_orbit);
    let logical_w = (aieyes_core::pets::BASE_WIDTH + pad_x * 2.0) as f32;
    let scale = width as f32 / logical_w;
    let dark = info["dark"] == true;
    let contrast = high_contrast();
    let selected_provider = info["orbitAgent"].as_str();
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
                pad_x as f32 + 6.0,
                pad_top as f32,
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
        if has_orbit {
            draw_orbit_nodes(
                graphics,
                font,
                orbit_agents,
                orbit_hover,
                selected_provider,
                orbit_elapsed_ms,
                dark,
                contrast,
            );
        }
        GdipDeleteGraphics(graphics);
    }
    if !canvas.is_null() {
        GdipDisposeImage(canvas.cast());
    }
    // The drawing spec of linux_pet.rs: translated label after a phase dot, optional count badge.
    let text = if sprite.frames.is_empty() {
        crate::localization::label("请导入桌宠").to_owned()
    } else if info["recovery"] == "failed" {
        crate::localization::label("恢复失败").to_owned()
    } else if info["recovery"] == "refreshing" {
        crate::localization::label("恢复中").to_owned()
    } else {
        crate::localization::text(label)
    };
    let (background, foreground) = if contrast {
        (system_argb(COLOR_WINDOW), system_argb(COLOR_WINDOWTEXT))
    } else if dark {
        (0xee202735, 0xffedf2fc)
    } else {
        (0xeef6f8fc, 0xff243049)
    };
    let dot = if contrast {
        foreground
    } else {
        let [r, g, b] = crate::phase_colors::rgb(info["phase"].as_str().unwrap_or("idle"), dark);
        0xff00_0000 | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
    };
    let badge = info["activeCount"]
        .as_u64()
        .filter(|count| *count > 0 && info["showCount"] == true)
        .map(|count| {
            let text = if count > 99 {
                "99+".to_owned()
            } else {
                count.to_string()
            };
            if contrast {
                (
                    text,
                    system_argb(COLOR_HIGHLIGHT),
                    system_argb(COLOR_HIGHLIGHTTEXT),
                )
            } else {
                (text, 0xff4d59d9, 0xffffffff)
            }
        });
    let spec = LabelSpec {
        text,
        background,
        foreground,
        dot,
        badge,
        has_orbit,
    };
    if label_cache
        .key
        .as_ref()
        .is_none_or(|(w, h, cached)| (*w, *h) != (width, height) || *cached != spec)
    {
        label_cache.pixels = render_label(width, height, font, &spec);
        label_cache.key = Some((width, height, spec));
    }
    // Both layers contain premultiplied BGRA; source-over preserves transparent edges.
    let pixels = std::slice::from_raw_parts_mut(bits, (width * height * 4) as usize);
    for (dst, src) in pixels
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(label_cache.pixels.as_chunks::<4>().0)
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
unsafe fn fill_round_rect(graphics: *mut GpGraphics, color: u32, rect: RectF, diameter: f32) {
    let (mut brush, mut path) = (null_mut(), null_mut());
    GdipCreateSolidFill(color, &mut brush);
    if GdipCreatePath(FillModeWinding, &mut path) == GDI_OK {
        let RectF {
            X: x,
            Y: y,
            Width: w,
            Height: h,
        } = rect;
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
}
unsafe fn draw_text(
    graphics: *mut GpGraphics,
    font: *mut GpFont,
    format: *mut GpStringFormat,
    text: &[u16],
    color: u32,
    rect: &RectF,
) {
    let mut brush = null_mut();
    GdipCreateSolidFill(color, &mut brush);
    GdipDrawString(
        graphics,
        PCWSTR(text.as_ptr()),
        text.len() as i32,
        font,
        rect,
        format,
        brush.cast(),
    );
    GdipDeleteBrush(brush.cast());
}
unsafe fn measure_width(
    graphics: *mut GpGraphics,
    font: *mut GpFont,
    format: *mut GpStringFormat,
    text: &[u16],
    layout: &RectF,
) -> Option<f32> {
    let mut measured = RectF::default();
    (GdipMeasureString(
        graphics,
        PCWSTR(text.as_ptr()),
        text.len() as i32,
        font,
        layout,
        format,
        &mut measured,
        null_mut(),
        null_mut(),
    ) == GDI_OK)
        .then_some(measured.Width)
}
unsafe fn argb(alpha: u8, [r, g, b]: [u8; 3]) -> u32 {
    (u32::from(alpha) << 24) | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
}

unsafe fn draw_ring_arc(
    graphics: *mut GpGraphics,
    cx: f32,
    cy: f32,
    radius: f32,
    line_width: f32,
    start_deg: f32,
    sweep_deg: f32,
    color: u32,
) {
    let mut pen = null_mut();
    if GdipCreatePen1(color, line_width, UnitPixel, &mut pen) == GDI_OK {
        GdipSetPenStartCap(pen, LineCapRound);
        GdipSetPenEndCap(pen, LineCapRound);
        GdipDrawArc(
            graphics,
            pen,
            cx - radius,
            cy - radius,
            radius * 2.0,
            radius * 2.0,
            start_deg,
            sweep_deg,
        );
        GdipDeletePen(pen);
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn draw_orbit_nodes(
    graphics: *mut GpGraphics,
    font: *mut GpFont,
    agents: &[aieyes_core::pets::PetOrbitAgent],
    hover: Option<usize>,
    selected_provider: Option<&str>,
    elapsed_ms: u64,
    dark: bool,
    contrast: bool,
) {
    use aieyes_core::pets::{ORBIT_NODE_SIZE, ORBIT_OVERFLOW_PROVIDER, orbit_slots};
    use aieyes_core::quota_ring::Ring;
    let slots = orbit_slots(agents.len());
    let reduced = reduced_motion();
    let mut format = null_mut();
    GdipCreateStringFormat(0, 0, &mut format);
    GdipSetStringFormatAlign(format, StringAlignmentCenter);
    GdipSetStringFormatLineAlign(format, StringAlignmentCenter);

    for (index, (agent, slot)) in agents.iter().zip(slots.iter()).enumerate() {
        let is_selected = selected_provider == Some(agent.provider.as_str());
        let is_hovered = hover == Some(index);
        let boost = if is_hovered || is_selected {
            1.08_f32
        } else {
            1.0
        };
        let cx = slot.cx as f32;
        let cy = slot.cy as f32;
        let radius = (ORBIT_NODE_SIZE as f32 / 2.0) * boost;

        // 1. Gentle breathing halo
        let (halo_alpha, halo_scale) =
            crate::pet_orbit::pulse_metrics(elapsed_ms, index, agent.phase, reduced);
        let pulse_rgb = crate::pet_orbit::pulse_rgb(agent, dark);
        let halo_r = radius * halo_scale as f32 + 1.5;
        let mut halo_pen = null_mut();
        if GdipCreatePen1(
            argb(
                (halo_alpha * 255.0).round().clamp(0.0, 255.0) as u8,
                pulse_rgb,
            ),
            2.4,
            UnitPixel,
            &mut halo_pen,
        ) == GDI_OK
        {
            GdipDrawEllipse(
                graphics,
                halo_pen,
                cx - halo_r,
                cy - halo_r,
                halo_r * 2.0,
                halo_r * 2.0,
            );
            GdipDeletePen(halo_pen);
        }

        // 2. Disc backdrop + subtle border
        let fill_color = if contrast {
            system_argb(COLOR_WINDOW)
        } else if dark {
            0xec232936
        } else {
            0xf0f8fafc
        };
        let mut brush = null_mut();
        GdipCreateSolidFill(fill_color, &mut brush);
        GdipFillEllipse(
            graphics,
            brush.cast(),
            cx - radius,
            cy - radius,
            radius * 2.0,
            radius * 2.0,
        );
        GdipDeleteBrush(brush.cast());

        let border_color = if is_selected {
            if dark { 0xffa3b3ff } else { 0xff5566d9 }
        } else if dark {
            0x668794ad
        } else {
            0x557b879d
        };
        let mut border_pen = null_mut();
        if GdipCreatePen1(
            border_color,
            if is_selected { 1.6 } else { 0.9 },
            UnitPixel,
            &mut border_pen,
        ) == GDI_OK
        {
            GdipDrawEllipse(
                graphics,
                border_pen,
                cx - radius,
                cy - radius,
                radius * 2.0,
                radius * 2.0,
            );
            GdipDeletePen(border_pen);
        }

        // 3. Quota ring
        let is_overflow = agent.provider == ORBIT_OVERFLOW_PROVIDER;
        let ring_width = 2.4_f32;
        let ring_r = radius - 2.2;
        let track_color = if dark { 0x40ffffff } else { 0x2b252528 };
        if !is_overflow {
            let ring = agent
                .quota
                .as_ref()
                .map(|q| &q.ring)
                .unwrap_or(&Ring::Empty);
            match ring {
                Ring::Duo { top, bottom } => {
                    let half = ((ring_width / 2.0 + 0.8) / ring_r).to_degrees();
                    let span = 180.0 - 2.0 * half;
                    draw_ring_arc(
                        graphics,
                        cx,
                        cy,
                        ring_r,
                        ring_width,
                        180.0 + half,
                        span,
                        track_color,
                    );
                    draw_ring_arc(
                        graphics,
                        cx,
                        cy,
                        ring_r,
                        ring_width,
                        180.0 - half,
                        -span,
                        track_color,
                    );
                    let top_rem = top.remaining.clamp(0.0, 100.0) as f32;
                    let bot_rem = bottom.remaining.clamp(0.0, 100.0) as f32;
                    if top_rem > 0.0 {
                        let c = argb(
                            255,
                            crate::pet_orbit::quota_arc_rgb(
                                &agent.display_provider,
                                top.remaining,
                                dark,
                            ),
                        );
                        draw_ring_arc(
                            graphics,
                            cx,
                            cy,
                            ring_r,
                            ring_width,
                            180.0 + half,
                            (span * top_rem / 100.0).max(1.0),
                            c,
                        );
                    }
                    if bot_rem > 0.0 {
                        let c = argb(
                            255,
                            crate::pet_orbit::quota_arc_rgb(
                                &agent.display_provider,
                                bottom.remaining,
                                dark,
                            ),
                        );
                        draw_ring_arc(
                            graphics,
                            cx,
                            cy,
                            ring_r,
                            ring_width,
                            180.0 - half,
                            -(span * bot_rem / 100.0).max(1.0),
                            c,
                        );
                    }
                }
                Ring::Single { top } => {
                    draw_ring_arc(
                        graphics,
                        cx,
                        cy,
                        ring_r,
                        ring_width,
                        0.0,
                        360.0,
                        track_color,
                    );
                    let rem = top.remaining.clamp(0.0, 100.0) as f32;
                    if rem > 0.0 {
                        let c = argb(
                            255,
                            crate::pet_orbit::quota_arc_rgb(
                                &agent.display_provider,
                                top.remaining,
                                dark,
                            ),
                        );
                        draw_ring_arc(
                            graphics,
                            cx,
                            cy,
                            ring_r,
                            ring_width,
                            270.0,
                            (360.0 * rem / 100.0).max(1.0),
                            c,
                        );
                    }
                }
                Ring::Coin { low, .. } => {
                    let rgb = if *low {
                        if dark {
                            [0xff, 0x92, 0x9c]
                        } else {
                            [0xb8, 0x32, 0x45]
                        }
                    } else {
                        crate::pet_orbit::provider_rgb(&agent.display_provider, dark)
                    };
                    draw_ring_arc(graphics, cx, cy, ring_r, 1.8, 0.0, 360.0, argb(160, rgb));
                }
                Ring::Empty => {
                    let mut dash_pen = null_mut();
                    if GdipCreatePen1(track_color, 1.6, UnitPixel, &mut dash_pen) == GDI_OK {
                        GdipSetPenDashStyle(dash_pen, DashStyleDot);
                        GdipDrawEllipse(
                            graphics,
                            dash_pen,
                            cx - ring_r,
                            cy - ring_r,
                            ring_r * 2.0,
                            ring_r * 2.0,
                        );
                        GdipDeletePen(dash_pen);
                    }
                }
            }
        }

        // 4. Center icon or +N overflow label
        if is_overflow {
            let label = format!("+{}", agent.overflow_providers.len());
            let utf16: Vec<u16> = label.encode_utf16().collect();
            let rect = RectF {
                X: cx - radius,
                Y: cy - radius,
                Width: radius * 2.0,
                Height: radius * 2.0,
            };
            let fg = if dark { 0xffedf2fc } else { 0xff243049 };
            if !font.is_null() {
                draw_text(graphics, font, format, &utf16, fg, &rect);
            }
        } else if let Some(icon) = crate::pet_icons::load(&agent.display_provider, dark) {
            let mut bitmap = null_mut();
            if GdipCreateBitmapFromScan0(
                icon.width as i32,
                icon.height as i32,
                (icon.width * 4) as i32,
                0xe200b,
                Some(icon.pixels.as_ptr()),
                &mut bitmap,
            ) == GDI_OK
            {
                let icon_size = 16.0 * boost;
                GdipDrawImageRectRect(
                    graphics,
                    bitmap.cast(),
                    cx - icon_size / 2.0,
                    cy - icon_size / 2.0,
                    icon_size,
                    icon_size,
                    0.0,
                    0.0,
                    icon.width as f32,
                    icon.height as f32,
                    UnitPixel,
                    std::ptr::null(),
                    0,
                    null_mut(),
                );
                GdipDisposeImage(bitmap.cast());
            }
        }

        // 5. Multi-session count pill
        if !is_overflow && agent.active_count > 1 {
            let count_text = if agent.active_count > 9 {
                "9+".to_owned()
            } else {
                agent.active_count.to_string()
            };
            let pill_w = if count_text.len() > 1 { 17.0 } else { 13.0 };
            let pill_h = 13.0;
            let pill = RectF {
                X: cx + radius - pill_w + 3.0,
                Y: cy + radius - pill_h + 3.0,
                Width: pill_w,
                Height: pill_h,
            };
            let pill_fill = if agent.phase == aieyes_core::sessions::Phase::NeedsInput {
                if dark { 0xffe0b062 } else { 0xff895712 }
            } else if dark {
                0xff5566d9
            } else {
                0xff4d59d9
            };
            fill_round_rect(graphics, pill_fill, pill, pill_h);
            if !font.is_null() {
                let utf16: Vec<u16> = count_text.encode_utf16().collect();
                draw_text(graphics, font, format, &utf16, 0xffffffff, &pill);
            }
        }
    }
    if !format.is_null() {
        GdipDeleteStringFormat(format);
    }
}

unsafe fn render_label(width: i32, height: i32, font: *mut GpFont, spec: &LabelSpec) -> Vec<u8> {
    let high = vec![0u8; (width * height * 4 * 9) as usize];
    let mut canvas = null_mut();
    let mut graphics = null_mut();
    let (pad_x, pad_top) = aieyes_core::pets::orbit_padding(spec.has_orbit);
    let logical_w = (aieyes_core::pets::BASE_WIDTH + pad_x * 2.0) as f32;
    let scale = width as f32 / logical_w;
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
        let text: Vec<u16> = spec.text.encode_utf16().collect();
        let mut format = null_mut();
        GdipCreateStringFormat(0, 0, &mut format);
        GdipSetStringFormatAlign(format, StringAlignmentCenter);
        GdipSetStringFormatLineAlign(format, StringAlignmentCenter);
        let area = height as f32 / scale - pad_top as f32 - 143.0;
        let layout = RectF {
            X: 0.0,
            Y: 0.0,
            Width: 110.0,
            Height: area,
        };
        // 9 px padding on both sides plus 10 px for the phase dot.
        let w = measure_width(graphics, font, format, &text, &layout).map_or(138.0, |measured| {
            (measured.ceil() + 28.0).clamp(30.0, 138.0)
        });
        let (x, y, h) = (
            pad_x as f32 + (144.0 - w) / 2.0,
            pad_top as f32 + 145.0,
            area - 4.0,
        );
        let pill = RectF {
            X: x,
            Y: y,
            Width: w,
            Height: h,
        };
        fill_round_rect(graphics, spec.background, pill, 20.0_f32.min(h));
        let mut brush = null_mut();
        GdipCreateSolidFill(spec.dot, &mut brush);
        GdipFillEllipse(graphics, brush.cast(), x + 9.5, y + h / 2.0 - 2.5, 5.0, 5.0);
        GdipDeleteBrush(brush.cast());
        GdipSetTextRenderingHint(graphics, TextRenderingHintAntiAliasGridFit);
        let rect = RectF {
            X: x + 19.0,
            Y: pad_top as f32 + 143.0,
            Width: w - 28.0,
            Height: area,
        };
        draw_text(graphics, font, format, &text, spec.foreground, &rect);
        if let Some((count, fill, ink)) = &spec.badge {
            let count: Vec<u16> = count.encode_utf16().collect();
            let bw = measure_width(graphics, font, format, &count, &layout)
                .map_or(20.0, |measured| (measured.ceil() + 10.0).max(20.0));
            let badge = RectF {
                X: pad_x as f32 + 144.0 - bw,
                Y: pad_top as f32 + 18.0,
                Width: bw,
                Height: 20.0,
            };
            fill_round_rect(graphics, *fill, badge, 20.0);
            draw_text(graphics, font, format, &count, *ink, &badge);
        }
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
        &[],
        None,
        0,
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
                    let spec = |text: &str, badge: Option<(String, u32, u32)>| LabelSpec {
                        text: text.into(),
                        background: 0xeef6f8fc,
                        foreground: 0xff243049,
                        dot: 0xff7a879d,
                        badge,
                        has_orbit: false,
                    };
                    let pixels = render_label(
                        width,
                        height,
                        font.font,
                        &spec("等待1 · 工作2\n思考1", None),
                    );
                    let short = render_label(width, height, font.font, &spec("空闲", None));
                    let counted = render_label(
                        width,
                        height,
                        font.font,
                        &spec("空闲", Some(("3".into(), 0xff4d59d9, 0xffffffff))),
                    );
                    let top_right = |buffer: &[u8]| {
                        let row = (28.0 * width as f64 / 144.0) as usize;
                        buffer[row * width as usize * 4..(row + 1) * width as usize * 4]
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .skip(width as usize * 3 / 4)
                            .any(|p| p[3] > 0)
                    };
                    assert!(
                        !top_right(&short) && top_right(&counted),
                        "count badge draws at the top right"
                    );
                    let extent = |buffer: &[u8]| {
                        let columns: Vec<_> = buffer
                            .as_chunks::<4>()
                            .0
                            .iter()
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
                        pixels
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .any(|p| p[3] > 240 && p[0] < 180),
                        "text strokes are visible"
                    );
                    assert!(
                        pixels
                            .as_chunks::<4>()
                            .0
                            .iter()
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
                    &[],
                    None,
                    0,
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
                    &[],
                    None,
                    0,
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
                    &[],
                    None,
                    0,
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
                    &[],
                    None,
                    0,
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
