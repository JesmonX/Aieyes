# Agent identity icons

Icons are bundled in `apps/desktop/web/provider-*.png` and copied into the macOS app resources by `scripts/build-macos.sh`. No runtime network requests are needed. All providers use transparent standalone marks without app tiles or colored background discs, displayed at 36 × 36 points/pixels. The originals retain their colors and proportions; PNG conversion and size reduction are only for local display. The monochrome OpenAI mark follows the light/dark foreground color.

| Provider | Official source |
| --- | --- |
| Codex (OpenAI monochrome Blossom) | https://cdn.openai.com/brand/OpenAI-Logos-2025.zip — `PNGs/OpenAI-black-monoblossom.png` |
| Claude Code (Claude mark) | https://assets.claude.com/95a868946ac8a31e5ff832e2899f294aa368b836.png |
| Antigravity / agy | https://antigravity.google/assets/image/brand/antigravity-icon__full-color.png |
| DeepSeek | https://api-docs.deepseek.com/img/favicon.svg |

Marks belong to their respective owners and identify the configured providers. agy retains its own visible name while sharing the Antigravity mark. Custom and unknown providers use a terminal symbol.
