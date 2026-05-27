import React, { useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import { listen } from "@tauri-apps/api/event";
import {
  Aperture,
  Bot,
  CheckCircle2,
  Eye,
  FileText,
  KeyRound,
  Loader2,
  Play,
  RotateCcw,
  Save,
  Sparkles,
  Target,
  Wand2
} from "lucide-react";
import type { GradeResult, ModelConfig, SelectionRect } from "./types";
import { invokeCommand, isTauri } from "./tauri";
import "./styles.css";

const defaultConfig: ModelConfig = {
  baseUrl: "https://api.openai.com/v1",
  model: "gpt-4.1-mini",
  hasApiKey: false
};

function formatRect(selection: SelectionRect | null) {
  if (!selection) return "未选择";
  return `${Math.round(selection.width)} x ${Math.round(selection.height)} px`;
}

function stringifyBreakdown(value: unknown) {
  if (!value || (Array.isArray(value) && value.length === 0)) return "暂无分项信息";
  if (typeof value === "string") return value;
  return JSON.stringify(value, null, 2);
}

function App() {
  const [config, setConfig] = useState<ModelConfig>(defaultConfig);
  const [apiKey, setApiKey] = useState("");
  const [selection, setSelection] = useState<SelectionRect | null>(null);
  const [preview, setPreview] = useState<string | null>(null);
  const [rubric, setRubric] = useState("");
  const [referenceEssay, setReferenceEssay] = useState("");
  const [maxScore, setMaxScore] = useState(60);
  const [result, setResult] = useState<GradeResult | null>(null);
  const [loading, setLoading] = useState<string | null>(null);
  const [status, setStatus] = useState("准备就绪");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isTauri()) {
      setStatus("浏览器预览模式：桌面能力需在 Tauri 中使用");
      return;
    }

    invokeCommand<ModelConfig>("load_model_config")
      .then((loaded) => setConfig(loaded))
      .catch((err) => setError(String(err)));

    let unlisten: (() => void) | undefined;
    listen<SelectionRect>("selection-confirmed", (event) => {
      const confirmedSelection = event.payload;
      setSelection(confirmedSelection);
      setStatus("选区已确认");
      setError(null);
      window.setTimeout(() => {
        invokeCommand<string>("capture_selection_preview", { selection: confirmedSelection })
          .then(setPreview)
          .catch((err) => setError(String(err)));
      }, 180);
    })
      .then((stop) => {
        unlisten = stop;
      })
      .catch((err) => setError(String(err)));

    return () => unlisten?.();
  }, []);

  const canGrade = useMemo(() => {
    return Boolean(
      selection &&
        config.baseUrl.trim() &&
        config.model.trim() &&
        config.hasApiKey &&
        rubric.trim() &&
        referenceEssay.trim() &&
        maxScore > 0
    );
  }, [config, maxScore, referenceEssay, rubric, selection]);

  async function saveConfig() {
    setLoading("config");
    setError(null);
    try {
      const saved = await invokeCommand<ModelConfig>("save_model_config", {
        input: {
          baseUrl: config.baseUrl,
          model: config.model,
          apiKey: apiKey.trim() ? apiKey.trim() : null
        }
      });
      setConfig(saved);
      setApiKey("");
      setStatus("模型配置已保存");
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(null);
    }
  }

  async function openSelectionWindow() {
    setLoading("selection");
    setError(null);
    try {
      await invokeCommand("open_selection_window");
      setStatus("正在框选作文区域");
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(null);
    }
  }

  async function refreshPreview() {
    if (!selection) return;
    setLoading("preview");
    setError(null);
    try {
      const image = await invokeCommand<string>("capture_selection_preview", { selection });
      setPreview(image);
      setStatus("选区预览已更新");
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(null);
    }
  }

  async function grade(mode: "single" | "auto") {
    if (!selection) {
      setError("请先框选作文区域");
      return;
    }

    setLoading(mode);
    setError(null);
    setResult(null);
    try {
      const graded = await invokeCommand<GradeResult>("grade_selection", {
        input: {
          selection,
          rubric,
          referenceEssay,
          maxScore,
          mode
        }
      });
      setResult(graded);
      setStatus(mode === "auto" ? "自动阅卷完成" : "单次阅卷完成");
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(null);
    }
  }

  return (
    <main className="app-shell">
      <header className="topbar">
        <div className="brand">
          <div className="brand-mark">
            <Bot size={22} aria-hidden />
          </div>
          <div>
            <h1>作文自动批改</h1>
            <p>{status}</p>
          </div>
        </div>
        <div className="status-pill" data-ready={canGrade}>
          {canGrade ? <CheckCircle2 size={16} /> : <Target size={16} />}
          <span>{canGrade ? "可阅卷" : "待补全"}</span>
        </div>
      </header>

      {error ? <div className="error-banner">{error}</div> : null}

      <section className="workspace-grid">
        <section className="panel config-panel">
          <div className="panel-title">
            <KeyRound size={18} aria-hidden />
            <h2>模型配置</h2>
          </div>
          <label>
            <span>Base URL</span>
            <input
              value={config.baseUrl}
              onChange={(event) => setConfig({ ...config, baseUrl: event.target.value })}
              placeholder="https://api.openai.com/v1"
            />
          </label>
          <label>
            <span>Model</span>
            <input
              value={config.model}
              onChange={(event) => setConfig({ ...config, model: event.target.value })}
              placeholder="gpt-4.1-mini"
            />
          </label>
          <label>
            <span>API Key</span>
            <input
              type="password"
              value={apiKey}
              onChange={(event) => setApiKey(event.target.value)}
              placeholder={config.hasApiKey ? "留空则保留已保存密钥" : "请输入 API Key"}
            />
          </label>
          <button className="primary-action" onClick={saveConfig} disabled={loading === "config"}>
            {loading === "config" ? <Loader2 className="spin" size={17} /> : <Save size={17} />}
            保存配置
          </button>
        </section>

        <section className="panel selection-panel">
          <div className="panel-title">
            <Aperture size={18} aria-hidden />
            <h2>作文选区</h2>
          </div>
          <div className="preview-frame">
            {preview ? (
              <img src={preview} alt="作文选区预览" />
            ) : (
              <div className="empty-preview">
                <Eye size={28} aria-hidden />
                <span>{formatRect(selection)}</span>
              </div>
            )}
          </div>
          <div className="selection-meta">
            <span>{formatRect(selection)}</span>
            <span>{selection ? `缩放 ${selection.scaleFactor.toFixed(2)}` : "等待框选"}</span>
          </div>
          <div className="button-row">
            <button onClick={openSelectionWindow} disabled={loading === "selection"}>
              <Target size={17} />
              框选作文
            </button>
            <button onClick={refreshPreview} disabled={!selection || loading === "preview"}>
              {loading === "preview" ? <Loader2 className="spin" size={17} /> : <RotateCcw size={17} />}
              更新预览
            </button>
          </div>
        </section>

        <section className="panel grading-panel">
          <div className="panel-title">
            <FileText size={18} aria-hidden />
            <h2>阅卷输入</h2>
          </div>
          <label>
            <span>满分</span>
            <input
              className="score-input"
              type="number"
              min={1}
              max={150}
              value={maxScore}
              onChange={(event) => setMaxScore(Number(event.target.value))}
            />
          </label>
          <label>
            <span>评分标准</span>
            <textarea
              value={rubric}
              onChange={(event) => setRubric(event.target.value)}
              placeholder="输入评分维度、扣分规则、等级描述"
            />
          </label>
          <label>
            <span>参考范文</span>
            <textarea
              value={referenceEssay}
              onChange={(event) => setReferenceEssay(event.target.value)}
              placeholder="输入参考范文或高分样例"
            />
          </label>
          <div className="button-row">
            <button className="primary-action" onClick={() => grade("single")} disabled={!canGrade || loading !== null}>
              {loading === "single" ? <Loader2 className="spin" size={17} /> : <Play size={17} />}
              单次阅卷
            </button>
            <button onClick={() => grade("auto")} disabled={!canGrade || loading !== null}>
              {loading === "auto" ? <Loader2 className="spin" size={17} /> : <Wand2 size={17} />}
              自动阅卷
            </button>
          </div>
        </section>

        <section className="panel result-panel">
          <div className="panel-title">
            <Sparkles size={18} aria-hidden />
            <h2>批改结果</h2>
          </div>
          {result ? (
            <div className="result-content">
              <div className="score-band">
                <strong>{result.score.toFixed(1)}</strong>
                <span>/ {result.maxScore}</span>
                <em>{result.level || "未分级"}</em>
              </div>
              <section>
                <h3>评语</h3>
                <p>{result.comments || "暂无评语"}</p>
              </section>
              <section>
                <h3>建议</h3>
                <p>{result.suggestions || "暂无建议"}</p>
              </section>
              <section>
                <h3>识别文本</h3>
                <p className="recognized-text">{result.extractedText || "模型未返回识别文本"}</p>
              </section>
              <section>
                <h3>分项</h3>
                <pre>{stringifyBreakdown(result.breakdown)}</pre>
              </section>
            </div>
          ) : (
            <div className="empty-result">
              <Sparkles size={30} aria-hidden />
              <span>阅卷结果会显示在这里</span>
            </div>
          )}
        </section>
      </section>
    </main>
  );
}

createRoot(document.getElementById("root")!).render(<App />);
