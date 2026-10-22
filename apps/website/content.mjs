export const en = {
  lang:'en', alternate:'简体中文', alternatePath:'/zh-CN/',
  title:'Aieyes — Your AI agents. One clear view.',
  description:'A desktop command center for AI coding agents. See usage, quotas, accounts, sessions and SSH resources together on macOS, Windows and Linux.',
  nav:['Features','Workflow','Download'], menu:'Open navigation', close:'Close', github:'View on GitHub', download:'Download Aieyes',
  eyebrow:'YOUR DESKTOP. A CLEARER PICTURE.',
  hero:['Your AI agents.','One clear view.'],
  intro:'Usage, quotas, accounts and remote machines. Aieyes brings the moving parts of your AI workflow into one calm desktop companion.',
  platforms:'macOS · Windows · Linux', preview:'Actual interface · Demo data', switchPlatform:'Preview platform', enlarge:'Enlarge preview',
  scene:'A home for your everyday AI workflow', sceneNote:'Real interface components, thoughtfully brought together.',
  ecosystem:'Bring the tools you already use.', ecosystemNote:'Native integrations for your agents. Capabilities vary by provider.',
  features:[
    {id:'overview',number:'01',kicker:'USAGE & QUOTAS',color:'violet',title:'Know what’s left.\nBefore the next prompt.',body:'Keep an eye on account quotas, reset windows, credits and today’s usage. Open the compact panel for a quick check, then return to what you were doing.',points:['Quota windows and reset times','Multiple accounts, one overview','A compact menu bar or desktop panel'],note:'Quota and balance details depend on the connected provider.'},
    {id:'analytics',number:'02',kicker:'USAGE ANALYTICS',color:'cyan',title:'See where\nyour tokens go.',body:'Follow your usage over time. Explore model mix, input and output, cache efficiency and an activity heatmap. Filter by agent, account or machine to find the detail that matters.',points:['Daily trends and model breakdowns','Cache reads and writes, in context','API-equivalent cost estimates'],note:'API-equivalent cost is an estimate based on model pricing.'},
    {id:'accounts',number:'03',kicker:'AGENTS, ACCOUNTS & APIs',color:'blue',title:'More tools.\nFewer places to check.',body:'Organize agent accounts and their machines in one place. Connect supported APIs, manage Codex sign-ins and switch between saved Codex accounts or API connections while keeping shared conversation history.',points:['Agent-specific account and API setup','Local and SSH data sources','Shared Codex history across account switches'],note:'API configuration and account-switching support vary by agent.'},
    {id:'sessions',number:'04',kicker:'SESSIONS & DESKTOP PETS',color:'pink',title:'A small companion.\nA useful signal.',body:'See when your agents are working, using tools or waiting for you. Let a desktop pet carry that state, or keep things quiet in the menu bar and system tray.',points:['Local and remote session status','Import a pet from a file, link or package','Switch pets from your own library'],note:'Featured pet: Yuexinmiao by kiffin.'},
    {id:'servers',number:'05',kicker:'SSH RESOURCE MONITORING',color:'green',title:'Your remote machines.\nA little closer.',body:'Check Linux servers over SSH alongside your agent activity. Keep CPU, memory, GPU, storage and network readings within reach, with the devices and detail you choose.',points:['CPU, memory and per-GPU readings','Filesystems, disk I/O and network','Choose metrics for each host'],note:'Resource monitoring shows current samples; long-term server metrics are not archived.'},
    {id:'wakeups',number:'06',kicker:'SCHEDULED WAKEUPS',color:'orange',title:'Give your workflow\na steady rhythm.',body:'Schedule small model calls at fixed times on your computer or an SSH Linux host. Review deployment, task status and run results without keeping another terminal open.',points:['Multiple daily times','Model and reasoning-effort selection','Task deployment and run history'],note:'Scheduled calls may consume quota. The target device must be available.'}
  ],
  workflowKicker:'LESS SETUP. MORE CLARITY.', workflowTitle:'From install to insight.',
  steps:[['Install Aieyes','Download the package for your operating system.'],['Connect your agents','Use the setup guide to select accounts and data sources.'],['Make it yours','Add an SSH host, choose your panel and pick a desktop pet.']],
  localTitle:'Your history. On your machine.', localBody:'Usage history is stored locally in SQLite. Provider queries and SSH connections use the sources you configure. You choose which agents and machines to connect.',
  localTags:['Local usage history','Your SSH connections','Source on GitHub'],
  downloadTitle:'A clearer desktop\nstarts here.', downloadBody:'Choose your platform. The latest release includes installers and checksums.', release:'Latest release',
  packages:[['macOS','macOS 14+','Apple Silicon & Intel · DMG'],['Windows','Windows x64','NSIS installer'],['Linux','Linux x64','AppImage · DEB · RPM']],
  faqTitle:'A few useful answers.',
  faq:[
    ['Does Aieyes replace my coding agents?','Aieyes works alongside your existing tools. It collects supported usage and session metadata, organizes accounts and APIs, and monitors your configured machines.'],
    ['What does API-equivalent cost mean?','It estimates the API value of recorded tokens using model pricing. It is useful for understanding usage; subscription charges and provider invoices are separate.'],
    ['Does every agent support the same features?','No. History, quota queries, API setup and session status depend on each agent and provider. For example, Cursor’s private request-level token database is not a supported history source. See the integration documentation for details.'],
    ['Can I use Aieyes in English?','Yes. Choose English or Simplified Chinese in Appearance settings, or use the language button in the panel. These screenshots use the product’s English interface with demo data. Windows images render the actual frontend.'],
    ['What should I know before installing?','macOS requires version 14 or later. Current packages use ad-hoc signing on macOS and do not have a Windows publisher certificate, so the operating system may ask you to allow the first launch. See the release notes for installation details.']
  ],
  docs:'Documentation', issues:'Report an issue', footer:'Keep your agents in sight.', attribution:'Pet artwork: Yuexinmiao by kiffin', notFound:'This page has wandered off.', back:'Back to Aieyes'
};

export const zh = {
  lang:'zh-CN', alternate:'English', alternatePath:'/',
  title:'Aieyes — AI Agent 工作状态，一眼看清。',
  description:'面向 AI 编程工作流的桌面工作台。用量、额度、账户、会话与 SSH 资源，一处掌握。支持 macOS、Windows 和 Linux。',
  nav:['主要功能','开始使用','下载'],menu:'打开导航',close:'关闭',github:'在 GitHub 查看',download:'下载 Aieyes',
  eyebrow:'在你的桌面，看清工作全貌。',hero:['所有 AI Agent，','一眼看清。'],
  intro:'用量、额度、多账号，还有远程机器。Aieyes 把 AI 工作流中分散的状态，带到你随时看得见的桌面。',
  platforms:'macOS · Windows · Linux',preview:'真实英文界面 · 演示数据',switchPlatform:'预览平台',enlarge:'放大预览',
  scene:'给日常 AI 工作流，一个清晰的入口',sceneNote:'基于当前真实界面组件渲染。',
  ecosystem:'接入你已经在用的工具。',ecosystemNote:'各 Agent 原生接入；具体能力随工具和服务商而异。',
  features:[
    {id:'overview',number:'01',kicker:'用量与额度',color:'violet',title:'下一次提问前，\n心里先有数。',body:'账户还剩多少额度，什么时候重置，今天用了多少 Token。打开紧凑面板就能看见，检查完继续手头的工作。',points:['额度窗口与重置时间','多个账户，统一查看','菜单栏与桌面紧凑面板'],note:'额度、余额等信息以对应服务商提供的能力为准。'},
    {id:'analytics',number:'02',kicker:'历史用量分析',color:'cyan',title:'每一份用量，\n都有迹可循。',body:'从每日趋势到模型分布，从输入输出到缓存效率，再到全年活动热力图。按 Agent、账户或机器筛选，把注意力放在你关心的部分。',points:['每日趋势与模型细分','缓存读取与写入明细','API 等价成本估算'],note:'API 等价成本依据模型价格估算，用于理解使用价值。'},
    {id:'accounts',number:'03',kicker:'Agent、账户与 API',color:'blue',title:'工具越来越多，\n入口可以更少。',body:'集中管理 Agent 账户和关联机器，配置支持的 API。管理 Codex 登录，在已保存的账户与 API 连接之间切换，并保留共用的对话历史。',points:['按 Agent 管理账户与 API','本机与 SSH 数据来源','Codex 切换账户后共用对话历史'],note:'API 配置及账户切换能力随 Agent 而异。'},
    {id:'sessions',number:'04',kicker:'会话状态与桌宠',color:'pink',title:'桌面上多一点陪伴，\n也多一份提醒。',body:'Agent 在工作、执行工具，还是正等你输入？让桌宠用状态告诉你，也可以选择安静常驻的菜单栏或系统托盘。',points:['本机与远程会话状态','从文件、链接或素材包导入桌宠','在自己的桌宠库中随时切换'],note:'展示桌宠：月薪喵，作者 kiffin。'},
    {id:'servers',number:'05',kicker:'SSH 服务器监控',color:'green',title:'远程机器，\n也能近在眼前。',body:'通过 SSH 查看 Linux 服务器资源，和 Agent 状态放在一起。CPU、内存、GPU、存储与网络，按主机选择需要的设备和细节。',points:['CPU、内存与多 GPU 读数','文件系统、磁盘 I/O 与网络','为每台主机选择监控指标'],note:'服务器监控展示当前采样，暂不保存长期资源历史。'},
    {id:'wakeups',number:'06',kicker:'定时唤醒',color:'orange',title:'让日常工作，\n多一点节奏感。',body:'在本机或 SSH Linux 主机上，按固定时刻发起轻量模型调用。查看部署状态、任务状态和运行结果，无需再留一个终端窗口。',points:['每天多个固定时刻','选择模型与推理强度','任务部署与运行记录'],note:'定时调用可能消耗额度，目标设备需要保持可用。'}
  ],
  workflowKicker:'从安装到看清，只需几步。',workflowTitle:'接入，然后开始。',
  steps:[['安装 Aieyes','下载与你的操作系统对应的安装包。'],['连接 Agent','跟随配置向导，选择账户和数据来源。'],['布置你的桌面','添加 SSH 主机，选择面板，再挑一个桌宠。']],
  localTitle:'你的历史，保存在你的机器。',localBody:'用量历史保存在本地 SQLite 中。额度查询与 SSH 连接使用你配置的来源，由你选择连接哪些 Agent 和机器。',localTags:['本地用量历史','使用你的 SSH 连接','在 GitHub 查看源码'],
  downloadTitle:'从这里，\n看清你的桌面。',downloadBody:'选择你的平台。最新发布页提供安装包及校验和。',release:'最新正式版本',
  packages:[['macOS','macOS 14+','Apple Silicon 与 Intel · DMG'],['Windows','Windows x64','NSIS 安装包'],['Linux','Linux x64','AppImage · DEB · RPM']],
  faqTitle:'你可能想了解。',
  faq:[
    ['Aieyes 会取代我的编程 Agent 吗？','Aieyes 与现有工具配合使用，采集支持的用量及会话元数据，管理账户与 API，并监控你配置的机器。'],
    ['API 等价成本是什么？','它按照模型价格估算已记录 Token 的 API 使用价值，帮助你理解用量。订阅费用和服务商实际账单另行计算。'],
    ['所有 Agent 都支持相同功能吗？','历史读取、额度查询、API 配置和会话状态取决于各 Agent 与服务商。例如，Cursor 私有数据库中的请求级 Token 尚未作为支持的历史来源。具体可查看接入文档。'],
    ['可以切换中英文吗？','可以。在外观设置中选择简体中文或 English，也可以使用面板上的语言按钮。展示图使用产品本身的英文界面和演示数据，Windows 展示来自实际前端。'],
    ['首次安装需要注意什么？','macOS 要求 14 或更新版本。当前 macOS 安装包使用 ad-hoc 签名，Windows 暂无发布者证书，首次打开可能需要在系统中允许。安装细节请查看发布说明。']
  ],
  docs:'使用与开发文档',issues:'反馈问题',footer:'把 Agent 的状态，放在眼前。',attribution:'桌宠原画：月薪喵 · kiffin',notFound:'这个页面暂时走丢了。',back:'返回 Aieyes'
};
