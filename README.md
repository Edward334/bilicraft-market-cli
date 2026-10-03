# Bilicraft AI Stock

Bilicraft AI Stock 是一个面向 Bilicraft Minecraft 虚拟股市的单用户 AI 交易服务。它把现有 Bilicraft Handheld 的微软登录和 Minecraft Java 协议行为，与 Bilicraft Handheld Stocks 的行情、K 线、`/bal`、`/invest` 命令解析和收益账本结合起来，提供一个 Rust 单体二进制、命令行和独立浏览器 Web UI。

## 当前能力

- Microsoft 消费者账号设备码登录，保存 refresh token 和 Minecraft 会话。
- Minecraft Java 1.21.11 / 协议 774 首期连接，精确支持协议表 767–776；包括状态 ping、登录加密、压缩、配置阶段、keep-alive、聊天组件/NBT 和命令发送。
- 行情、健康状态、K 线、可用股数、自选股、持仓、成本、已实现/未实现盈亏和每日收益快照。
- OpenAI 兼容接口：可配置 `base_url`、请求路径、模型、超时、重试和 API key。
- AI 只能返回结构化 `buy`、`sell`、`hold` 或 `no_trade` 决策；实际下单前重新读取资金、持仓和公司状态。
- AI 预算、总资产上限、交易时间段、自动化时间段、单笔限额、每日交易次数、每日亏损、持仓数、冷却时间和熔断。
- 独立浏览器 Web UI：Material Web、Material color utilities、轻量 K 线图表、桌面/移动端布局、深色模式和 `prefers-reduced-motion`。
- QQ 官方 Bot 扫码绑定：`@tencent-connect/qqbot-connector` 只用于一次性扫码取得 AppID/AppSecret，之后 Rust 管理 token、Gateway、频道/群/私聊推送和只读查询。
- SQLite 审计、凭据 `0600` 文件、SSE 事件流和无需 Docker 的部署脚本。

## 快速启动

需要 Rust/Cargo。Web 前端构建需要 Node.js 18+，QQ 扫码绑定也需要 Node.js 18+。

```bash
cp config.example.toml config.toml
./scripts/install.sh
./scripts/login.sh
./scripts/start.sh
```

浏览器打开 `http://127.0.0.1:8787`。首次进入 Web UI 时必须设置访问密码；密码以 PBKDF2-SHA256 哈希保存，不写入明文。之后每次访问需要登录，设置页可以修改访问密码。默认 AI 和自动化交易关闭。

可用命令：

```text
bilicraft-ai-stock init
bilicraft-ai-stock login
bilicraft-ai-stock connect|disconnect|status
bilicraft-ai-stock market list|health|kline <company_id>
bilicraft-ai-stock portfolio show|refresh|history
bilicraft-ai-stock watchlist list|add|remove
bilicraft-ai-stock trade buy|sell <company_id> <shares>
bilicraft-ai-stock ai status|run-once|ask|pause|resume
bilicraft-ai-stock risk show|validate
bilicraft-ai-stock qq setup|status|test
bilicraft-ai-stock audit
```

## 配置

复制 `config.example.toml` 后修改：

- `[server]`：Minecraft 地址、端口、协议号和玩家名。协议填 `0` 时先执行状态 ping。
- `[market]`：现有股市接口地址和同步周期。
- `[ai]`：OpenAI 兼容接口。API key 放在 `data/secrets.json`，不要写进 `config.toml`。
- `[risk]`：AI 固定预算、总资产百分比、交易/自动化窗口、单笔金额、每日损失、每日次数、冷却和最大持仓。
- `[qq]`：QQ API 地址、推送格式和目标。目标使用频道 ID、群 OpenID 或用户 OpenID。
- `[web]`：Web 监听地址。默认只监听本机，公网使用 Nginx/Caddy 反代。Web UI 访问密码不在 `config.toml` 中，保存在 `data/secrets.json` 的哈希字段中。

日常使用只需在「AI」页面填写投入本金（例如 `200000`）、检查间隔，然后点击「开始自动交易」或「停止」。开始会检查 API Key、Minecraft 连接和本金，并启用 AI 接口。默认不限制单笔金额、持仓公司数或每日次数；若需要，可在「设置 → AI 资金 → 可选限制」打开额外限制。时间窗口留空表示全天，当前高级窗口按 UTC 计算。

AI 当前资金 = 累计投入本金 + AI 已实现收益；可继续买入金额 = 当前资金 − AI 持仓成本，并以实际账户现金为上限。盈利卖出会增加可用资金，亏损卖出会减少资金。修改本金表示改变累计投入总额，不会清空历史收益。人工交易与无法确认归属的历史记录不会自动计为 AI 收益；新的 AI 持仓和收益单独记账，服务重启后保留。人工卖出混合持仓时，账本先消耗非 AI 持仓，超出的部分才计入 AI 卖出。

停止会阻止后续自动运行以及尚未提交的自动订单，已发送给 Minecraft 的订单仍可能完成。「运行一次」和策略问答可能下单，不是预览。自动调度直接使用共享执行器，不依赖 Web 登录 Cookie。AI 返回的价格、余额和持仓只作为分析输入，执行器会从 Minecraft 服务器读取事实。

## QQ 官方扫码绑定

```bash
./target/release/bilicraft-ai-stock qq setup
```

程序调用项目内的 `scripts/qq-setup.mjs`，该脚本使用官方 `startQrConnect` 回调接口显示二维码链接。用手机 QQ 扫码并选择机器人后，凭据通过私有 JSON Lines 管道回到 Rust，并原子写入 `data/secrets.json`；AppSecret 不打印到终端、Web 或普通日志。

然后在 `config.toml` 填写至少一个目标。首版主动推送交易结果、AI 收益/亏损、当前持仓、逐股浮盈亏、风控拒绝和连接异常。QQ 入站只提供 `/status`、`/holdings`、`/pnl` 只读查询以及一次性 `/bind` 目标绑定，不提供 QQ 交易控制。

## 部署与备份

```bash
./scripts/init.sh
./scripts/install.sh
./scripts/start.sh
./scripts/backup.sh
./scripts/update.sh
```

`systemd/bilicraft-ai-stock.service.example` 可复制到 systemd。脚本不依赖 Docker；公网部署时只需让反向代理指向本地 Web 端口。备份包含 SQLite、配置和受保护的 `data/secrets.json`，应把备份文件放到受控位置。

## 开发验证

```bash
cargo fmt --all
cargo check
cargo test
npm --prefix web run build
npm --prefix web test
```

目前离线测试覆盖 VarInt/帧、AES-CFB8、NBT、协议 palette、登录配置阶段和 keep-alive、资金/价格/交易解析、QQ 模板/路由/凭据过期、AI 风控预算和时间窗口。真实 Microsoft 登录、Bilicraft 服务器交易、QQ 扫码、QQ Gateway、公网反代和不同服务器版本仍需在目标环境验收。

## 来源

请阅读 [THIRD_PARTY.md](THIRD_PARTY.md) 查看两个源项目的基准提交、复用路径和许可证说明。
