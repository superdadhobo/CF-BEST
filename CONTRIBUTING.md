# 参与贡献

感谢你愿意改进 CF Best！本文说明如何报告问题、提交代码以及发布新版本。

## 报告问题

- 先在 [Issues](https://github.com/wintopic/CF-BEST/issues) 中搜索，确认问题还没有被报告过。
- 请附上系统版本、应用版本（状态栏右下角）、复现步骤，以及运行日志中的相关内容。日志位于数据目录下的 `runtime.log`（Windows：`%APPDATA%\CFBest`，macOS：`~/Library/Application Support/CFBest`）。
- **不要在 Issue 中粘贴订阅链接、UUID 或节点地址。**

## 开发环境

- Rust 1.96+（`rustup update stable`）
- Windows：Visual Studio 生成工具（“使用 C++ 的桌面开发”）
- macOS：Xcode 命令行工具（`xcode-select --install`）

```bash
cargo run            # debug 构建
cargo run --release  # 接近发布版的性能
```

调试用的环境变量（`CFBEST_PAGE`、`CFBEST_THEME`、`CFBEST_DEMO` 等）见 README 的「从源码构建」一节。

## 提交代码

1. Fork 仓库，从 `main` 创建分支。
2. 保持改动聚焦：一个 PR 只做一件事。
3. 提交前运行与 CI 相同的检查：

   ```bash
   cargo fmt --check
   cargo clippy --locked --all-targets -- -D warnings
   cargo test --locked
   ```

4. 改动界面时，请在 PR 中附上前后截图。
5. 代码风格与现有代码保持一致：注释用中文，说明「为什么」而不是「做了什么」。界面文字用简体中文书面语。

### 代码结构约定

- `src/core/` 是与界面无关的业务逻辑（扫描、测速、验证、存储），应能独立测试，不依赖 GPUI。
- `src/app/` 是应用状态与页面，`src/ui/` 是可复用组件。
- 平台相关代码放在 `#[cfg(target_os = "...")]` 后面，并为其他平台提供空实现。能用 `cfg!()` 表达式时优先用它，这样所有平台都会编译检查到这段代码。
- `src/theme_tokens.rs` 由 `tools/sync_qoder_tokens.py` 生成，不要手改。

## 发布新版本

发布由 GitHub Actions 自动完成（[`release.yml`](.github/workflows/release.yml)）：

1. 修改 `Cargo.toml` 中的 `version`，运行一次 `cargo build` 更新 `Cargo.lock`，并在 [CHANGELOG.md](CHANGELOG.md) 中记录改动。
2. 提交后打上同版本号的标签并推送：

   ```bash
   git tag v0.2.0
   git push origin main --tags
   ```

3. 工作流会校验标签与 `Cargo.toml` 的版本一致，分别构建 Windows x64 的 `.exe` 与 macOS universal 的 `.dmg`，生成 `SHA256SUMS.txt`，并创建 GitHub Release（附带自动生成的更新说明）。

在 Actions 页面手动运行 Release 工作流只会构建产物，不会发布。

## 许可

提交代码即表示你同意以 [MIT 许可证](LICENSE) 发布你的贡献。
