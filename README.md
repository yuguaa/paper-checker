# paper-checker

作文自动批改桌面 Agent。应用基于 Tauri v2 + React + TypeScript + Rust，支持 Windows exe 和 Apple Silicon macOS 应用打包。

## 功能

- 输入兼容 OpenAI SDK 的 `baseUrl`、`apiKey`、`model`，并可设置 `Memory Key`
- 框选试卷作文区域，确认后在屏幕上保留透明取景框
- 输入评分标准、参考范文、满分
- 使用视觉模型直接识别作文图片并返回分数、评语和改进建议
- 支持人工订正分数，并按 `Memory Key` 隔离写入本地 `memory-wiki.md`
- 保存评分记录、截图文件、模型返回和分段耗时，便于排查识别慢或评分异常
- GitHub Actions 构建 Windows 与 macOS arm64 产物

## 开发

```bash
npm install
npm run dev
```

本机运行 Tauri 需要安装 Rust 与对应平台的 Tauri 系统依赖。

开发版 Windows 如果没有 MSVC 链接工具，需要安装 Visual Studio Build Tools 的 C++ 工具链。

## 构建

```bash
npm run tauri -- build
```

GitHub Actions 会在 `windows-latest` 与 `macos-latest` 上分别构建 Windows NSIS `.exe` 和 Apple Silicon macOS `.app/.dmg`。
