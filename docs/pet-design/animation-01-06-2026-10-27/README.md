# 01 / 06 完整桌宠动作 · 2026-10-27

基于用户选择的「01 折耳巡游兽」和「06 墨滴工匠」制作。角色参考只使用 fresh-exploration-2026-10-27 中的两张候选板。

## 直接查看与导入

- [交互动画预览](preview.html)：双角色、九动作、明暗背景、暂停、慢放、时间滑块和减少动态效果；HTML 自包含，可离线打开。
- [动作对照图](preview-first-frames.png)。
- [01 导入包](01-folded-ear-scout.codex-pet.zip)、[01 透明图集](01-folded-ear-scout/spritesheet.png)、[01 全动作轮播](01-folded-ear-scout/all-actions.webp)。
- [06 导入包](06-inkdrop-artisan.codex-pet.zip)、[06 透明图集](06-inkdrop-artisan/spritesheet.png)、[06 全动作轮播](06-inkdrop-artisan/all-actions.webp)。
- 各角色目录内的 animations/ 含九个独立动作的无损动态 WebP；frames/ 含全部 57 个独立透明 PNG 帧。

包内仅有 pet.json 与 spritesheet.png，可在 Aieyes 的桌宠导入器选择 ZIP。此任务只在隔离数据目录验证导入，没有替换用户当前选用的桌宠。

## 规范核对

**9 行动作、8 列最大帧数，不是 8 个动作。** 当前 Aieyes 会话状态投影使用其中 8 行；左跑行保留以完整覆盖 Codex 动作图集。

采用 Codex v1 动作布局：1536 × 1872 RGBA PNG；每格 192 × 208；8 × 9。每只 57 个有效帧，15 个未使用格全透明。v2 在这些动作之外增加两行共 16 个视线方向格，本次交付的是完整 v1 动作图集，不含 v2 视线扩展。

核对来源：

1. 当前仓库 crates/core/src/pets.rs：实际状态到动作行、帧数的投影。
2. docs/core-contracts.json 的 petFrameDurations：Aieyes 各端共享的逐帧时间规范。
3. apps/desktop/web/pet-animation.js：预览直接使用这个实际播放器的快照。
4. 本机官方 ChatGPT/Codex 桌面应用 26.1021.31345，build 21828，/Applications/ChatGPT.app/Contents/Resources/app.asar 中 webview/assets/app-initial-feb30b196d11.js 的动作定义：九种动作及对应行、帧数和基础时长与上述规范一致。只读取核对，未更改官方应用或导入到官方账户。
5. [OpenAI 官方 Pets 文档](https://learn.chatgpt.com/docs/pets)：公开上传规范为透明 PNG/WebP、1536 × 1872、20 MiB 以内；具体九行动作由本地实现核实。

## 完整映射及时间

行号从 0 开始；有效帧放在该行最左侧，其余格透明。

| 行 | Codex 动作 | 帧数 | 每帧毫秒 | 周期 | Aieyes 映射 |
|---|---|---:|---|---:|---|
| 0 | idle | 6 | 280,110,110,140,140,320 | 1100 ms | 空闲；已中断复用 |
| 1 | running-right | 8 | 120,120,120,120,120,120,120,220 | 1060 ms | Working / 进行中 |
| 2 | running-left | 8 | 120,120,120,120,120,120,120,220 | 1060 ms | 当前会话映射未使用，方向动作保留 |
| 3 | waving | 4 | 140,140,140,280 | 700 ms | NeedsInput / 等待输入 |
| 4 | jumping | 5 | 140,140,140,140,280 | 840 ms | Complete / 已完成 |
| 5 | failed | 8 | 140,140,140,140,140,140,140,240 | 1220 ms | Failed / 执行失败 |
| 6 | waiting | 6 | 150,150,150,150,150,260 | 1010 ms | Unknown；无可选会话时状态不全 |
| 7 | running | 6 | 120,120,120,120,120,220 | 820 ms | Tool / 执行工具 |
| 8 | review | 6 | 150,150,150,150,150,280 | 1030 ms | Thinking / 思考中 |

Aieyes 以这些时长持续循环当前状态；官方客户端的慢 idle 倍率与默认非循环退回 idle 策略不是 Aieyes 当前策略。素材遵循同一行布局，播放节奏由客户端控制；ZIP manifest 不擅自增加会被忽略的帧数/时长字段。

## 区分度设计

- 空闲：安静呼吸与眨眼；不以失败、睡眠姿态代替。
- 右跑、左跑：朝向清晰，低重心前倾、四肢交替；01 左跑单独补画为完整奔跑周期。
- 等待输入：从第一帧就抬起一只手/前爪招呼用户。
- 完成：双手举起、开心表情、起跳与落地；首尾帧均明确开心。
- 失败：低头、耳朵或笔锋下垂、受挫姿态；不会使用庆祝或打字画面。
- 待确认：坐姿、双手靠下、左右张望。
- 工具工作：面朝键盘、双手操作；与奔跑明确不同。
- 思考：01 托腮抬眼；06 手持小本子并托腮。06 另行补画，避免小尺寸下与等待混淆。
- 每行动作第一帧具备该状态特征，兼容减少动态效果。

## 制作与复现

角色绘制与修正使用内置 image_gen，不使用已有其他宠物素材。完整提示词见 prompts.md 和 refinement-prompts.md。sources/ 保留原始生成结果供追溯。

生成图未天然满足精确尺寸与分格，因此 build.cjs 只做技术处理：按透明连通区域拆分原始帧、保留抗锯齿透明边缘、按序列固定比例归一化、基线与跳跃高度定位、排入标准网格。没有用重复同一帧凑数；每只 57 帧文件校验值均不同。所有角色及补画均由 image_gen 绘制，技术脚本不绘制角色。

每个角色的 animation.json 保存每帧源图、裁切范围、缩放、目标坐标、SHA-256、时长及状态映射。

从仓库根目录运行：

~~~sh
node docs/pet-design/animation-01-06-2026-10-27/build.cjs
node docs/pet-design/animation-01-06-2026-10-27/verify-and-preview.cjs
node docs/pet-design/animation-01-06-2026-10-27/export-animations.cjs
node docs/pet-design/animation-01-06-2026-10-27/check-browser.cjs
node docs/pet-design/animation-01-06-2026-10-27/package-and-check.cjs
~~~

使用仓库 apps/desktop/node_modules 已有的 sharp 与 playwright，无新增应用依赖。

## 验证及边界

- validation.json：精确尺寸、RGBA、57 非空帧、15 全透明格、格边 4 px 无非透明像素、帧唯一性、每个时长边界及循环回绕与真实 JS 播放器相符。
- browser-check.json：36 个 Canvas 明暗对照；时间推进全部更新；减少动态效果均取各行动作第一帧；无页面错误。
- import-check.json：对最终 ZIP 使用本地 target/debug/aieyes-core 的真实 pets.import，在临时隔离数据目录导入；两只均返回 error:null、8 列、9 行、192 × 208 单格。
- animations/ 的编码结果反读验证帧数与每帧毫秒一致。
- 已查看完整图集、首帧与中间时刻的明暗背景渲染，针对发现的缺帧、庆祝表情、左跑末帧及思考辨识度做过修正。
- 本次验证覆盖素材、导入和浏览器播放；未执行 Windows/Linux/macOS 原生桌宠窗口的交互验收，也未在官方 Codex/ChatGPT 中实际安装。帧数/映射符合是确定性检查；视觉自然度和最终产品选型仍可通过预览继续评审。
