# paper-checker

作文自动批改桌面 Agent。应用基于 Tauri v2 + React + TypeScript + Rust，支持 Windows exe 和 Apple Silicon macOS 应用打包。

## 功能

- 输入兼容 OpenAI SDK 的 `baseUrl`、`apiKey`、`model`
- 框选试卷作文区域，确认后截取半透明选区
- 输入评分标准、参考范文、满分
- 使用视觉模型直接识别作文图片并返回分数、评语和改进建议
- GitHub Actions 构建 Windows 与 macOS arm64 产物

## 开发

```bash
npm install
npm run dev
```

本机运行 Tauri 需要安装 Rust 与对应平台的 Tauri 系统依赖。

## 构建

```bash
npm run tauri -- build
```

GitHub Actions 会在 `windows-latest` 与 `macos-latest` 上分别构建 Windows NSIS `.exe` 和 Apple Silicon macOS `.app/.dmg`。
