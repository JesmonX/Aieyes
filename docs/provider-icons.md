# Agent identity icons

Icons are bundled as transparent 144 × 144 PNGs in `apps/desktop/web/provider-*.png` and copied into macOS resources by `scripts/build-macos.sh`. No runtime downloads are needed. Provider assets, theme variants and agent aliases are declared in `docs/core-contracts.json` and generated for Web and Swift.

The shared container is 36 × 36 points/pixels, with smaller containers in selectors. Each mark is cropped to its visible artwork, preserves its proportions, and receives its own transparent padding. Optical size is adjusted for stroke density and shape rather than equalizing image dimensions or bounding boxes. The reviewed longer-side fractions are recorded in `provider-icon-sources.json`; both clients display the same padded assets.

| Agent | Official artwork |
| --- | --- |
| Codex | [OpenAI monochrome Blossom](https://cdn.openai.com/brand/OpenAI-Logos-2025.zip), `PNGs/OpenAI-black-monoblossom.png` |
| Claude Code | [Claude mark](https://assets.claude.com/95a868946ac8a31e5ff832e2899f294aa368b836.png) |
| Antigravity / agy | [Antigravity icon](https://antigravity.google/assets/image/brand/antigravity-icon__full-color.png) |
| DeepSeek | [DeepSeek mark](https://api-docs.deepseek.com/img/favicon.svg), shared by the balance and Harness adapters |
| Cursor | [Official brand assets](https://cursor.com/brand), `General Logos/Cube/SVG/CUBE_2D_LIGHT.svg` |
| Grok | [Navigation mark](https://grok.com), both `path#mark` elements |
| ZCode | [Navigation mark](https://zcode.z.ai/en), `viewBox="0 0 256 218"` |
| Qoder | [Official SVG](https://img.alicdn.com/imgextra/i4/O1CN018ikLCF1sGya2c3YY4_!!6000000005740-55-tps-206-206.svg), separate rounded tile removed |
| OpenCode | [Official light/dark standalone marks](https://github.com/anomalyco/opencode/tree/dev/packages/console/app/src/asset/brand), without the favicon tile |
| Kimi Code | [Official homepage](https://kimi.com) CSS `--sidebar-logo`, K and blue accent isolated from the tile |
| CodeBuddy | [Official SVG](https://codebuddy-1328495429.cos.accelerate.myqcloud.com/web/ide/logo.svg), foreground robot and eyes in brand violet; background and clipping removed |
| Pi | [Official multicolor mark](https://pi.dev/logo-auto.svg) |
| Oh My Pi | [Official connector mark](https://github.com/can1357/oh-my-pi/blob/main/assets/icon.svg), light/dark foreground variants |
| ChatGPT Work | Same OpenAI Blossom asset as Codex |
| GitHub Copilot | [LobeHub mark](https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@latest/icons/copilot.svg), interim until official artwork is reviewed |
| Cline | [LobeHub mark](https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@latest/icons/cline.svg), interim |
| Kilo Code | [LobeHub mark](https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@latest/icons/kilocode.svg), interim |
| Roo Code | [LobeHub mark](https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@latest/icons/roocode.svg), interim |
| Kiro | [LobeHub color mark](https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@latest/icons/kiro-color.svg), tile kept, interim |
| Trae / Trae CN | [LobeHub color mark](https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@latest/icons/trae-color.svg), interim |
| WorkBuddy | [LobeHub color mark](https://cdn.jsdelivr.net/npm/@lobehub/icons-static-svg@latest/icons/workbuddy-color.svg), interim |

Codex, ChatGPT Work, Cursor, Grok, ZCode, Qoder, Copilot, Cline, Kilo Code and Roo Code follow the light/dark foreground color. Kimi, OpenCode and Oh My Pi have separate light/dark PNGs to preserve their colored accents or tonal detail. Claude, Antigravity, DeepSeek, CodeBuddy, WorkBuddy, Kiro, Trae and Pi retain their brand colors. Custom or unknown providers use the shared terminal fallback. Marks belong to their respective owners.
