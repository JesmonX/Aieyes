//! Embedded provider icons decoded to premultiplied BGRA for Windows GDI+ and Linux Cairo.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

#[derive(Clone)]
pub struct DecodedIcon {
    pub width: u32,
    pub height: u32,
    /// Premultiplied BGRA (compatible with Win32 GDI+ PixelFormat32bppPARGB and Cairo ARgb32 LE).
    pub pixels: Vec<u8>,
}

fn icon_key(provider: &str, dark: bool) -> Option<&'static str> {
    if dark {
        match provider {
            "opencode" => return Some("opencode-dark"),
            "kimi" => return Some("kimi-dark"),
            "omp" => return Some("omp-dark"),
            _ => {}
        }
    }
    match provider {
        "codex" => Some("codex"),
        "claude" => Some("claude"),
        "antigravity" => Some("antigravity"),
        "deepseek" | "deepseek-harness" => Some("deepseek"),
        "zcode" => Some("zcode"),
        "qoder" | "qoder-cn" => Some("qoder"),
        "grok" => Some("grok"),
        "cursor" => Some("cursor"),
        "opencode" => Some("opencode"),
        "kimi" => Some("kimi"),
        "codebuddy" => Some("codebuddy"),
        "pi" => Some("pi"),
        "omp" => Some("omp"),
        "chatgpt-work" => Some("chatgpt-work"),
        "workbuddy" => Some("workbuddy"),
        "copilot" => Some("copilot"),
        "cline" => Some("cline"),
        "kilo" => Some("kilo"),
        "roo" => Some("roo"),
        "kiro" => Some("kiro"),
        "trae" | "trae-cn" => Some("trae"),
        _ => None,
    }
}

fn is_monochrome(provider: &str) -> bool {
    matches!(
        provider,
        "codex"
            | "zcode"
            | "qoder"
            | "qoder-cn"
            | "grok"
            | "cursor"
            | "chatgpt-work"
            | "copilot"
            | "cline"
            | "kilo"
            | "roo"
    )
}

fn raw_png(key: &str) -> Option<&'static [u8]> {
    match key {
        "antigravity" => Some(include_bytes!("../../web/provider-antigravity.png")),
        "chatgpt-work" => Some(include_bytes!("../../web/provider-chatgpt-work.png")),
        "claude" => Some(include_bytes!("../../web/provider-claude.png")),
        "cline" => Some(include_bytes!("../../web/provider-cline.png")),
        "codebuddy" => Some(include_bytes!("../../web/provider-codebuddy.png")),
        "codex" => Some(include_bytes!("../../web/provider-codex.png")),
        "copilot" => Some(include_bytes!("../../web/provider-copilot.png")),
        "cursor" => Some(include_bytes!("../../web/provider-cursor.png")),
        "deepseek" => Some(include_bytes!("../../web/provider-deepseek.png")),
        "grok" => Some(include_bytes!("../../web/provider-grok.png")),
        "kilo" => Some(include_bytes!("../../web/provider-kilo.png")),
        "kimi" => Some(include_bytes!("../../web/provider-kimi.png")),
        "kimi-dark" => Some(include_bytes!("../../web/provider-kimi-dark.png")),
        "kiro" => Some(include_bytes!("../../web/provider-kiro.png")),
        "omp" => Some(include_bytes!("../../web/provider-omp.png")),
        "omp-dark" => Some(include_bytes!("../../web/provider-omp-dark.png")),
        "opencode" => Some(include_bytes!("../../web/provider-opencode.png")),
        "opencode-dark" => Some(include_bytes!("../../web/provider-opencode-dark.png")),
        "pi" => Some(include_bytes!("../../web/provider-pi.png")),
        "qoder" => Some(include_bytes!("../../web/provider-qoder.png")),
        "roo" => Some(include_bytes!("../../web/provider-roo.png")),
        "trae" => Some(include_bytes!("../../web/provider-trae.png")),
        "workbuddy" => Some(include_bytes!("../../web/provider-workbuddy.png")),
        "zcode" => Some(include_bytes!("../../web/provider-zcode.png")),
        _ => None,
    }
}

fn decode_png(bytes: &[u8], tint: Option<[u8; 3]>) -> Option<DecodedIcon> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(
        png::Transformations::EXPAND | png::Transformations::STRIP_16 | png::Transformations::ALPHA,
    );
    let mut reader = decoder.read_info().ok()?;
    let mut raw = vec![0u8; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut raw).ok()?;
    raw.truncate(info.buffer_size());
    let mut rgba = match info.color_type {
        png::ColorType::Rgba => raw,
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
            for chunk in raw.as_chunks::<3>().0 {
                out.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
            }
            out
        }
        png::ColorType::GrayscaleAlpha => {
            let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
            for chunk in raw.as_chunks::<2>().0 {
                out.extend_from_slice(&[chunk[0], chunk[0], chunk[0], chunk[1]]);
            }
            out
        }
        png::ColorType::Grayscale => {
            let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
            for &g in &raw {
                out.extend_from_slice(&[g, g, g, 255]);
            }
            out
        }
        _ => return None,
    };
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let alpha = u16::from(pixel[3]);
        let (r, g, b) = if let Some([tr, tg, tb]) = tint {
            (u16::from(tr), u16::from(tg), u16::from(tb))
        } else {
            (
                u16::from(pixel[0]),
                u16::from(pixel[1]),
                u16::from(pixel[2]),
            )
        };
        // Premultiplied BGRA
        pixel[0] = ((b * alpha + 127) / 255) as u8;
        pixel[1] = ((g * alpha + 127) / 255) as u8;
        pixel[2] = ((r * alpha + 127) / 255) as u8;
        pixel[3] = alpha as u8;
    }
    Some(DecodedIcon {
        width: info.width,
        height: info.height,
        pixels: rgba,
    })
}

type IconCacheKey = (&'static str, Option<[u8; 3]>);
type IconCache = HashMap<IconCacheKey, DecodedIcon>;

pub fn load(provider: &str, dark: bool) -> Option<DecodedIcon> {
    let key = icon_key(provider, dark)?;
    let tint = is_monochrome(provider).then_some(if dark {
        [0xed, 0xf2, 0xfc]
    } else {
        [0x24, 0x30, 0x49]
    });
    let cache_key = (key, tint);
    static CACHE: OnceLock<Mutex<IconCache>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock()
        && let Some(existing) = guard.get(&cache_key)
    {
        return Some(existing.clone());
    }
    let decoded = decode_png(raw_png(key)?, tint)?;
    if let Ok(mut guard) = cache.lock() {
        guard.insert(cache_key, decoded.clone());
    }
    Some(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_all_known_provider_icons_in_light_and_dark_themes() {
        let providers = [
            "codex",
            "claude",
            "antigravity",
            "deepseek",
            "zcode",
            "qoder",
            "qoder-cn",
            "grok",
            "cursor",
            "opencode",
            "kimi",
            "deepseek-harness",
            "codebuddy",
            "pi",
            "omp",
            "chatgpt-work",
            "workbuddy",
            "copilot",
            "cline",
            "kilo",
            "roo",
            "kiro",
            "trae",
            "trae-cn",
        ];
        for provider in providers {
            for dark in [false, true] {
                let icon = load(provider, dark)
                    .unwrap_or_else(|| panic!("missing icon: {provider} dark={dark}"));
                assert!(icon.width > 0 && icon.height > 0);
                assert_eq!(icon.pixels.len(), (icon.width * icon.height * 4) as usize);
                assert!(
                    icon.pixels
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .all(|p| p[0] <= p[3] && p[1] <= p[3] && p[2] <= p[3]),
                    "premultiplied alpha invariant failed for {provider}"
                );
            }
        }
    }
}
