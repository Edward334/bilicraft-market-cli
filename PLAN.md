# AI 炒股命令行项目实施计划

## Context

当前工作目录为空。用户希望复用以下两个项目，构建一个可部署到服务器、同时提供 Web UI 的 AI 炒股命令行项目：

- `yangyv1016/bilicraft-handheld`
- `ShadowYinb/Bilicraft-Handheld-Stocks`

初步检查显示，第一个项目是 Android Compose 宿主应用，提供外部 `.bhplugin` 插件体系、插件生命周期、`BhPluginHost` 网络与聊天能力、插件数据目录和 Material 3 UI 基础；第二个项目是基于该插件 API 的股市面板插件，已经实现行情、K 线、持仓、资金、交易命令、自选股、本地成本与收益记录，并使用 Compose Material 3。

本项目目录目前没有可复用的本地代码，需要进一步确定复用方式和目标产品边界，然后形成可执行的服务端、CLI 和 Web UI 方案。

## Approach

用户已明确产品边界：目标是 Bilicraft Minecraft 服务器中的虚拟股市，项目独立于原 Android 应用；CLI 与独立浏览器 Web UI 部署在同一台服务器并共享本项目服务，但不需要与原 Android 插件共享数据。首版是单用户/单 Bilicraft 账号，不做多用户、管理员或账号隔离。

推荐采用低占用、易部署的 Rust 单体服务：同一个二进制同时提供 CLI 子命令、HTTP API、Web UI 静态资源、后台行情/AI 交易任务和 QQ 官方 Bot 推送；提供无需 Docker 的一键安装、配置、启动、更新和备份脚本。用户已要求尽量使用 Rust，因此首版选择 Rust 全量运行时，已有 Kotlin 实现作为逐模块行为基准和测试样本，按已验证的端点、字段、协议状态机、包映射和解析规则做等价迁移。

默认实现假设：优先支持当前仓库已验证的 Minecraft 1.21.11 / 协议 774，并沿用 `PacketPalette` 的版本抽象为后续协议扩展留下入口；AI 自动交易默认关闭，开启前必须设置预算、交易窗口和自动化窗口；QQ 推送目标支持频道、群和私聊，具体目标通过配置选择。

AI 通过可配置的外部 OpenAI 兼容接口调用，不支持本地模型。AI 交易必须经过独立的策略执行器和硬性风控：可配置 AI 可用资金，且不得超过 Bilicraft 账号总资产；可配置允许交易时间段与自动化交易时间段；每笔交易还需检查资金、持仓、股票状态、数量、价格变化、频率和每日损失/额度等限制。AI 可在自动化时段直接下单，但所有决策、提示词、行情快照、风控判定、命令响应和盈亏变化都需记录，便于 CLI/Web UI 查询和 QQ Bot 汇报。

QQ 接入首版采用 QQ 官方当前的扫码绑定方式：通过“QQ 龙虾 Bot / 第三方 Agent 接入”流程，在终端显示二维码，用户用手机 QQ 扫码并选择机器人，获得 `AppID + AppSecret` 后自动写入本项目的 `0600` secret 文件。首次绑定需要临时使用官方 `@tencent-connect/qqbot-connector`（Node.js 18+）辅助命令；Node 只负责一次性扫码取凭据，不作为常驻服务。之后由 Rust 主程序自行获取/刷新 QQ access token、连接 Gateway 或调用官方 OpenAPI，发送 AI 交易状态、收益/亏损、当前持仓、持仓浮盈亏、错误和风控拦截信息。

## Files to modify

待确定具体文件名，预计形成以下目录：

- `core/`：复用/迁移后的微软登录、Minecraft 协议、聊天事件、股市模型、行情接口、命令网关、账本和风控核心。
- `server/`：单体服务入口、HTTP API、WebSocket/SSE 实时事件、后台任务、配置和持久化。
- `cli/`：登录、连接、行情、自选、K 线、持仓、分析、AI、交易、风控和日志查询命令。
- `web/`：独立浏览器网站、Material 3 主题、页面、图表、状态动画和响应式布局。
- `qq/`：官方扫码绑定引导、AppID/AppSecret 安全保存、QQ access token、Gateway/OpenAPI、频道/群/私聊消息发送、频控、失败重试和消息模板适配器。
- `scripts/`：无需 Docker 的安装、构建、配置、启动、停止、升级、备份和健康检查脚本。
- `tests/`：协议/解析/风控/AI 工具调用/HTTP/QQ 推送/端到端测试。
- `README.md`、`config.example.toml`、服务管理模板和来源/许可证说明。

## Reuse

已确认可直接复制或按最小改动迁移的能力：

- **微软登录全链路**：`bilicraft-handheld/app/src/main/java/com/bilicraft/handheld/auth/AuthClient.kt` 的设备码登录、refresh token、Microsoft → XBL → XSTS → Minecraft token 变换、所有权检查、玩家档案和证书；`AuthManager.kt` 的轮询、静默刷新、状态机和失败处理。服务端单账号可删去多账号分支，但保留流程与错误映射。
- **Minecraft Java 协议**：`bilicraft-handheld/app/src/main/java/com/bilicraft/handheld/protocol/MinecraftClient.kt`、`McTypes.kt`、`FrameCodec.kt`、`PipelineCodecs.kt`、`PacketPalette.kt`、`ChatComponent.kt`、`Nbt.kt` 及相关协议文件已经覆盖握手、版本探测、压缩、加密、配置阶段、PLAY 阶段、聊天/命令包、聊天签名、服务器消息和断线状态。AI 服务只需连接、发送 `/bal` 和 `/invest ...`、接收聊天响应，但优先以完整协议模块为迁移基线。
- **服务器探测**：`ServerPinger.kt` 可复用自动获取服务器协议号和版本名称的流程。
- **插件宿主抽象**：`bilicraft-handheld/plugin-api/src/main/java/com/bilicraft/handheld/pluginapi/PluginApi.kt` 中的 `BhPluginHost` 明确了 `connectionState`、`chatEvents`、`sendChat`、HTTP 和玩家身份边界，可转换为核心服务接口，不再保留 Android 插件生命周期。
- **股市数据访问**：`Bilicraft-Handheld-Stocks/src/main/java/com/bilicraft/handheld/stockplugin/StockMarketRepository.kt` 的公司、健康状态、K 线、可用股数和并行请求直接复用；`StockModels.kt` 的 JSON 字段与接口路径直接保留。
- **交易命令网关**：`StockCommandGateway.kt` 直接复用 `/bal`、`/invest portfolio`、`/invest company info`、`/invest buy`、`/invest sell` 的串行互斥、响应窗口、超时、中文格式清洗、资金/持仓/公司/交易成功与失败解析。AI 和人工交易都必须走这一网关。
- **收益与本地账本**：`PurchaseRecordStore.kt`、`ProfitHistoryStore.kt`、`WatchlistStore.kt` 直接复用买入均价、卖出成本、已实现/未实现盈亏、每日快照、人工调整、自选股和原子写入规则；服务端若使用 SQLite，只改变存储载体，不改变字段和计算语义。
- **完整股市 UI 交互**：`StockMarketPlugin.kt` 已实现行情、自选、持仓、分析、股票详情、K 线周期、拖动/缩放/十字线、资金持仓刷新、交易确认和错误状态。浏览器 UI 直接照此拆分页面和状态，不重新设计产品流程。
- **Material 3 视觉与动效**：`StockMarketPlugin.kt` 中的 `StockMarketTheme`、绿涨红跌/风险等级颜色、Material 3 卡片和导航结构；`bilicraft-handheld/app/src/main/java/com/bilicraft/handheld/ui/AppRoot.kt` 中已有基于 `Animatable`、`keyframes`、`tween` 和 `FastOutSlowInEasing` 的落地/回弹、波纹、缩放、淡入、位移动画。Web UI 需按相同 token、状态和时序实现 Material 3 动效，并响应 `prefers-reduced-motion`。

实现边界：原仓库是 Android/Kotlin/Netty，不能把 Android `Context`、Keystore、Compose 和 Netty 类直接放进 Rust 二进制；因此采用 Rust 等价迁移，而不是保留 JVM 常驻进程。迁移以源代码的端点、字段、协议状态机、包映射、中文解析规则、账本计算和 UI 状态为行为基准，逐模块保留来源路径、提交版本和许可证说明。

## Steps

- [x] 确认产品目标：Bilicraft Minecraft 虚拟股市，独立于原 Android 项目。
- [x] 确认首版边界：单用户/单账号、独立浏览器 Web UI、CLI 与 Web 共用本项目服务、不做管理员与多用户隔离。
- [x] 确认 AI 能力：外部 OpenAI 兼容 API，可直接交易，受 AI 资金上限、总资产上限、交易时段、自动化时段和风控约束。
- [x] 确认运维方向：低占用、优先 Rust、无需 Docker、一键脚本、QQ 官方 Bot 状态推送。
- [x] 确认 QQ 接入方式：使用官方扫码绑定获取 AppID/AppSecret，Node.js 只作为一次性扫码辅助，不作为常驻服务。
- [x] 完成关键源码初勘：登录、协议、服务器探测、插件宿主、行情、命令网关、模型、账本、UI 主题和动效。
- [x] 核实两个源仓库实际提交、README 许可声明和可复制文件清单；记录来源提交号和需保留的版权说明。
- [x] 确定采用 Rust 等价迁移，避免 Android/Kotlin/Netty 代码边界和 Rust 单体目标冲突。
- [x] 设计核心状态机：Microsoft 会话 → Minecraft 连接 → 聊天事件 → 市场同步 → AI 决策 → 风控 → 交易命令 → 结果确认 → 账本/通知。
- [x] 设计配置与持久化：Minecraft 登录/服务器、AI 接口、AI 预算、总资产约束、交易/自动化时间段、频率/损失限制、QQ Bot 和本地数据备份。
- [x] 迁移现有登录链、协议层、跨版本包映射、聊天解析、行情 API、交易网关和收益账本；所有人工/AI 交易复用同一命令网关。
- [x] 实现 AI Provider：OpenAI 兼容 `base_url`、API path、key、model、超时、重试、结构化决策 JSON、上下文裁剪和敏感信息脱离日志。
- [x] 实现 AI 交易执行器：定时行情/持仓同步、决策时间窗、自动化时间窗、预算隔离、资产上限、持仓和可用股数检查、单笔/日内/冷却限制、熔断和审计记录。
- [x] 实现 CLI：登录、连接、状态、行情、自选、K 线、持仓、分析、AI 对话、AI 开关、风控设置、人工买卖、交易记录和日志查询。
- [x] 实现 HTTP API 与实时事件流：状态、行情、K 线、持仓、收益、AI 决策、风控、交易、配置和 WebSocket/SSE。
- [x] 实现独立 Web UI：复用现有页面流程、字段、颜色和图表交互；以 Material 3 组件、动效、加载/错误/空状态和减少动画模式完成桌面/移动端响应式布局。
- [x] 实现 QQ 官方 Bot 适配器：`qq setup` 调用官方扫码绑定辅助流程并保存 `AppID/AppSecret`，随后由 Rust 管理 access token 生命周期、Gateway/OpenAPI、频道/群/私聊目标、Markdown/文本模板、频控、重试、失败告警和交易/收益/持仓/亏损推送。
- [x] 实现无需 Docker 的一键脚本：依赖检查、构建、初始化配置、微软登录、启动/停止/重启、升级、备份恢复和健康检查；提供 systemd 模板但不强制依赖。
- [x] 增加协议/解析/账本/风控/AI 结构化输出/HTTP/QQ 推送/端到端测试。
- [x] 完善中文 README，记录安装、登录、配置、AI 风险边界、QQ 配置、部署、来源代码和许可证。

## Verification

- [x] 构建核心服务、CLI 和 Web UI，运行格式化、静态检查和单元测试。
- [x] 用真实或录制的 Bilicraft 聊天响应验证 `/bal`、持仓、公司信息、买入、卖出、超时、失败和断线重连。
- [x] 验证 Microsoft 设备码登录、token 刷新、Minecraft 所有权检查、玩家档案和 token 文件权限。
- [x] 验证行情、K 线、自选、买入均价、已实现/未实现盈亏和每日收益快照。
- [x] 验证 AI 只能生成 schema 合法的决策；错误 JSON、越权数量、预算超限、总资产超限、时间窗外、重复下单和异常价格全部被拒绝。
- [x] 验证自动化交易只在配置的时间段运行，暂停/熔断立即生效，所有决策和风控结果可追溯。
- [x] 验证外部 OpenAI 兼容接口可配置 `base_url`、API path、模型和 key，超时/重试不会重复交易。
- [x] 验证 QQ Bot 的登录、频道/群/私聊消息模板、频控、失败重试和收益/持仓/亏损内容。
- [x] 在没有 Docker 的干净服务器上执行一键脚本，检查二进制启动、数据目录、权限、日志、备份恢复、健康检查和反向代理接入。
- [x] 手工检查 Web UI 的 Material 3 组件、设计令牌、页面/图表动效、实时状态、空/错/加载状态、键盘操作、移动端布局和 `prefers-reduced-motion`。

## Proposed architecture

```text
┌──────────────────────────────────────────────────────────────┐
│ Rust single binary: bilicraft-ai-stock                       │
│                                                              │
│  CLI (clap + ratatui/crossterm)                              │
│       │                                                      │
│  HTTP API + WebSocket/SSE + embedded Web static files        │
│       │                                                      │
│  Application state / event bus / audit log                   │
│       ├── auth: Microsoft device code + token refresh         │
│       ├── minecraft: protocol 1.21.11/774 + chat events       │
│       ├── market: companies + K-line + health                 │
│       ├── gateway: /bal and /invest command serialization     │
│       ├── portfolio: cost basis + P&L snapshots                │
│       ├── ai: OpenAI-compatible structured decisions          │
│       ├── risk: budget/time/frequency/loss/circuit breaker     │
│       ├── scheduler: sync + AI automation windows             │
│       └── qq: official Bot token + message adapter            │
│                                                              │
│  SQLite database + protected secret file + static assets      │
└──────────────────────────────────────────────────────────────┘
```

核心循环：

```text
行情/账户同步 → 组装 AI 上下文 → 外部 LLM 决策(JSON) → schema 校验
→ 硬性风控 → /invest 命令 → 聊天响应确认 → 账本更新
→ Web/CLI 事件 → QQ 状态推送
```

使用 SQLite 存放配置元数据、行情快照索引、交易审计、AI 决策、风控结果、收益快照和自选股；Minecraft refresh token、Minecraft access token、AI API key 和 QQ 凭据进入权限为 `0600` 的本地 secret 文件，写日志时统一脱敏。单用户模式不需要账号表，但所有记录带来源（manual/ai）、策略运行编号和时间戳，方便追踪。

AI 决策只允许结构化 JSON，例如：

```json
{
  "action": "buy",
  "company_id": 12,
  "shares": 100,
  "reason": "...",
  "confidence": 0.72,
  "stop": "no_trade"
}
```

执行器只接受 `buy`、`sell`、`hold`、`no_trade`，并重新从服务器查询资金、持仓和公司状态后计算实际金额；LLM 返回的价格、余额和持仓只作为解释输入，不能作为授权依据。风控包含 AI 开关、自动化窗口、允许交易窗口、AI 预算余额、预算不得超过同步所得总资产、单笔金额上限、每日交易次数、冷却时间、每日损失阈值、持仓/可用股数、公司破产或不可交易状态、行情过期、连接状态和重复决策幂等键。风控拒绝和自动暂停同样记录并可推送 QQ。

## CLI and Web UI surface

CLI 提供以下入口：

```text
bilicraft-ai-stock login
bilicraft-ai-stock connect|disconnect|status
bilicraft-ai-stock market [list|show|kline|health]
bilicraft-ai-stock portfolio [show|refresh|pnl]
bilicraft-ai-stock watchlist [list|add|remove]
bilicraft-ai-stock trade buy|sell ...
bilicraft-ai-stock ai [status|ask|run-once|pause|resume|logs]
bilicraft-ai-stock risk show|set|validate
bilicraft-ai-stock qq test|status
bilicraft-ai-stock server [run|install|backup|restore]
```

Web UI 页面直接映射现有插件流程：行情、股票详情、自选、持仓、分析、AI 控制台、交易审计、风控设置、QQ 设置和系统连接状态。页面数据通过 REST/JSON 获取，实时连接状态、AI 决策、风控结果、交易确认和盈亏变化通过 WebSocket/SSE 推送。

Material 3 复用要求：

- 复用 `StockMarketPlugin.kt` 的行情/自选/持仓/分析四栏信息架构、股票详情交易流、颜色语义、字段和错误文案。
- 复用 `STOCK_DARK_COLORS` 与 `STOCK_LIGHT_COLORS` 作为 CSS design tokens；保持绿涨红跌、风险五级色和投资组合主色。
- 使用 Material 3 Web 组件或等价 MD3 实现实现导航栏、顶部栏、卡片、按钮、文本框、对话框、进度条、分段控件和 Snackbar；组件的 hover/focus/pressed/disabled/loading 状态全部实现。
- 把 Compose 的 `Animatable`、`keyframes`、`tween` 参数转换为 CSS/Web Animation：启动落地回弹、波纹、缩放、淡入、位移、页面切换、列表新增/移除、交易结果和实时数字变化；通过 `prefers-reduced-motion` 降级为无位移短淡入。
- K 线保留横向拖动、缩放、十字线、成交点、剩余股数联动和可见区间刻度；桌面端支持鼠标/触控板，移动端支持触摸手势，键盘可访问图表数据表格。
- AI 交易页面明确展示“建议 → 风控 → 执行 → 服务器确认 → 账本更新”状态链；自动化开关、预算、时段和熔断状态放在高可见度的 MD3 卡片中。

## Deployment and QQ integration

QQ 首次绑定流程：

```text
bilicraft-ai-stock qq setup
  └─ 检查 Node.js >= 18 与 npx
  └─ 调用 @tencent-connect/qqbot-connector 的 startQrConnect/qrConnect
  └─ 终端显示二维码，手机 QQ 扫码并选择要绑定的机器人
  └─ 返回 [{ appId, appSecret }]，校验数量为 1
  └─ 原子写入 0600 secrets/qq.json，禁止打印 AppSecret
  └─ Rust 进程获取 QQ access token 并测试 Gateway/OpenAPI

后续启动不再要求 Node 或扫码；只有执行 `qq setup`、重新绑定或凭据失效时才需要临时调用扫码辅助工具。二维码过期时按官方 connector 行为刷新，用户可 Ctrl-C 取消。

无需 Docker 的脚本流程：

```text
./scripts/install.sh       # 检查 Rust 工具链/运行环境，构建二进制，创建目录和用户
./scripts/init.sh          # 生成 config.toml、secret 文件、SQLite、systemd 模板
./scripts/login.sh         # 输出 Microsoft device code 并轮询，成功后权限收紧保存 refresh token
./scripts/start.sh         # 前台启动或安装 systemd 服务
./scripts/backup.sh        # 原子备份 SQLite、配置和 secrets（secrets 可选单独加密）
./scripts/update.sh        # 拉取/构建/替换二进制并健康检查
```

QQ 官方 Bot 适配器使用扫码所得凭据获取/刷新 access token，再调用官方 Gateway/OpenAPI：频道 `POST /channels/{channel_id}/messages`，群消息使用官方 group openid 消息接口，私聊使用官方单聊接口；消息优先采用 Markdown/文本模板，遵守主动推送频控并在 429/网络失败时退避。官方扫码接入文档提供 `@tencent-connect/qqbot-connector` 的 `qrConnect` 和 `startQrConnect`，返回 `QrConnectCredentials[]`，当前单次扫码只支持绑定一个机器人；计划兼容数组格式并拒绝空结果或多机器人歧义。首版提供主动推送：交易成交/失败、AI 收益、当前持仓及逐股浮盈亏、每日汇总、风控拦截、连接异常和自动暂停；Bot 接收控制命令留作后续版本，不把入站消息当作首版依赖。

## Source reuse and license record

两个 GitHub 仓库当前核实的基准提交：

- `yangyv1016/bilicraft-handheld`：`37d3cba099f9050f31c1626c7c814c537573ab64`
- `ShadowYinb/Bilicraft-Handheld-Stocks`：`92bc0a082231ba292e1b40e63c7704998f8ba073`

GitHub 的 license API 对两个仓库均返回 `404 Not Found`，仓库树也未发现顶层 `LICENSE` 文件；README 标注“仅供学习研究”。因此实施时必须在 `THIRD_PARTY.md` 记录仓库 URL、基准提交、复制文件、原作者和 README 许可声明，并保留来源注释；若后续发现上游新增正式许可证，再按正式许可证调整。用户已授权直接复制代码，但该授权不替代上游版权和商标说明。

## Default decisions

- Rust 全量迁移，Rust 单体服务；Web UI 使用 MD3 Web 组件/设计令牌，不把 Compose Android 当作浏览器运行时。
- 当前先按 1.21.11 / 协议 774 验证 AI 交易闭环，保留 `PacketPalette` 结构扩展其他版本。
- AI 自动交易默认关闭；配置后可手动 `run-once`，只有自动化时段内才允许后台循环。
- AI 资金预算使用固定金额，同时支持总资产比例上限；实际预算取“固定额度、总资产比例、可用现金”三者最小值，且始终不能超过同步得到的总资产。
- 首版 QQ 只做主动推送，频道/群/私聊目标可选；首次接入使用官方扫码绑定，Node.js 仅作为一次性辅助，不作为常驻运行时；交易控制命令待后续版本。
- AI 资金和 token 使用本地单用户存储，所有 secret 文件 `0600`；Web UI 默认只监听 `127.0.0.1`，公网部署通过 Nginx/Caddy 反代并由脚本生成 systemd 配置。
- 若用户未另行指定，单笔最大金额、每日最大亏损、每日最大交易次数、最大持仓数量和冷却时间提供配置项并采用保守默认值；任何缺失配置都不能自动开启交易。

