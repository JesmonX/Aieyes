# Agent identity icons

Icons are bundled in `apps/desktop/web/provider-*.png` and copied into the macOS app resources by `scripts/build-macos.sh`. No runtime network requests are needed. The icon container is 36 × 36 points/pixels. Prefer official transparent standalone marks when available; retain existing assets when an appropriate standalone version cannot be confirmed. The originals retain their proportions; PNG conversion and size reduction are only for local display. Monochrome Codex, Cursor, Grok and ZCode marks follow the light/dark foreground color.

| Provider | Official source |
| --- | --- |
| Codex (OpenAI monochrome Blossom) | https://cdn.openai.com/brand/OpenAI-Logos-2025.zip — `PNGs/OpenAI-black-monoblossom.png` |
| Claude Code (Claude mark) | https://assets.claude.com/95a868946ac8a31e5ff832e2899f294aa368b836.png |
| Antigravity / agy | https://antigravity.google/assets/image/brand/antigravity-icon__full-color.png |
| DeepSeek | https://api-docs.deepseek.com/img/favicon.svg |
| Cursor (2D standalone cube) | https://cursor.com/brand — official ZIP, `General Logos/Cube/SVG/CUBE_2D_LIGHT.svg` |
| Grok (standalone mark) | https://grok.com — the two `path#mark` elements in the home navigation logo |
| ZCode (standalone mark) | https://zcode.z.ai/en — navigation logo SVG, `viewBox="0 0 256 218"` |

Cursor, Grok and ZCode were checked on 2026-10-17 and rendered with transparent padding into 144 px PNGs. Qoder's official standalone SVG still contains a rounded tile; OpenCode's official brand asset is a rectangular mark. Their existing icons are retained. Sources for those retained assets remain in `provider-icon-sources.json`.

Marks belong to their respective owners and identify the configured providers. agy retains its own visible name while sharing the Antigravity mark. Custom and unknown providers use a terminal symbol at two thirds of the surrounding icon container size.
