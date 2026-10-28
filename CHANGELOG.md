# 更新日志

本项目的所有重要改动都记录在这里。格式参考 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.0/)，版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

## [0.1.0] - 2026-10-28

首个公开版本。

### 新增

- 自动优选：一键或每天定时执行「扫描 → 测速 → 验证」，按段池轮转，保存并导出 Top N；错过时间自动补跑。
- 扫描 IP：支持 CIDR、单个 IP、`IP:端口`、ASN 前缀与推荐段，显示机房与握手延迟。
- 测速排行：逐个下载测速（初筛 + 多轮精测），附速度分布与「延迟—速度」图表。
- 可用性验证：VLESS + WebSocket 隧道验证；没有节点时做 TLS 基础验证。
- 段池优化与历史记录（高级模式）。
- 托盘常驻、任务栏进度、开机自启（Windows）；macOS universal 安装包。
- 4 种主题风格 × 浅色 / 深色 / 跟随系统，界面缩放，全键盘操作。

[0.1.0]: https://github.com/wintopic/CF-BEST/releases/tag/v0.1.0
