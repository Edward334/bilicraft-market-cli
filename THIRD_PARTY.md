# 第三方来源与复用说明

本项目按用户授权复用以下公开项目的实现思路和源代码片段。上游仓库当前未发现顶层 `LICENSE` 文件，README 标注“仅供学习研究”；发布前应再次核实上游许可证并保留版权说明。

## Bilicraft Handheld

- Repository: https://github.com/yangyv1016/bilicraft-handheld
- Baseline commit: `37d3cba099f9050f31c1626c7c814c537573ab64`
- Reused as behavior baseline: Microsoft device-code authentication, Minecraft Java protocol state machine, protocol palette, framing/compression/encryption, chat component/NBT parsing, server ping and player session flow.
- Original paths: `app/src/main/java/com/bilicraft/handheld/auth/`, `app/src/main/java/com/bilicraft/handheld/protocol/`, `app/src/main/java/com/bilicraft/handheld/storage/`.

## Bilicraft Handheld Stocks

- Repository: https://github.com/ShadowYinb/Bilicraft-Handheld-Stocks
- Baseline commit: `92bc0a082231ba292e1b40e63c7704998f8ba073`
- Reused as behavior baseline: market API paths and models, K-line intervals, Chinese `/bal` and `/invest` response parsing, purchase cost basis, profit snapshots, watchlist semantics and Material 3 stock dashboard information architecture.
- Original paths: `src/main/java/com/bilicraft/handheld/stockplugin/StockMarketRepository.kt`, `StockCommandGateway.kt`, `StockModels.kt`, `PurchaseRecordStore.kt`, `ProfitHistoryStore.kt`, `WatchlistStore.kt`, `StockMarketPlugin.kt`.

## QQ official integration

- QQ Bot official documentation: https://bot.q.qq.com/wiki/
- QR connector: https://www.npmjs.com/package/@tencent-connect/qqbot-connector
- The one-shot Node helper uses the official connector only to obtain QR-bound `AppID` and `AppSecret`. The long-lived service is Rust and uses the official QQBot API/Gateway.

Minecraft and Microsoft are trademarks of their respective owners. This project is for the Bilicraft virtual market integration and does not claim affiliation with Microsoft, Mojang or Tencent.
