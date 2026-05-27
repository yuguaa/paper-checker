import React, { useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import { listen } from "@tauri-apps/api/event";
import {
  Aperture,
  Bot,
  CheckCircle2,
  FileText,
  KeyRound,
  Loader2,
  PencilLine,
  Play,
  Save,
  Sparkles,
  Target,
  Wand2
} from "lucide-react";
import type { GradeRecord, GradeResult, MemoryWikiSaveResult, ModelConfig, SelectionRect } from "./types";
import { invokeCommand, isTauri } from "./tauri";
import "./styles.css";

const defaultConfig: ModelConfig = {
  baseUrl: "https://api.openai.com/v1",
  model: "gpt-4.1-mini",
  memoryKey: "default",
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

function formatMs(value: number | undefined) {
  if (!Number.isFinite(value)) return "-";
  if ((value ?? 0) >= 1000) return `${((value ?? 0) / 1000).toFixed(1)}s`;
  return `${Math.round(value ?? 0)}ms`;
}

function App() {
  const [config, setConfig] = useState<ModelConfig>(defaultConfig);
  const [apiKey, setApiKey] = useState("");
  const [selection, setSelection] = useState<SelectionRect | null>(null);
  const [rubric, setRubric] = useState("");
  const [referenceEssay, setReferenceEssay] = useState("");
  const [maxScore, setMaxScore] = useState(60);
  const [result, setResult] = useState<GradeResult | null>(null);
  const [correctedScore, setCorrectedScore] = useState("");
  const [correctionNote, setCorrectionNote] = useState("");
  const [memoryMessage, setMemoryMessage] = useState<string | null>(null);
  const [records, setRecords] = useState<GradeRecord[]>([]);
  const [loading, setLoading] = useState<string | null>(null);
  const [status, setStatus] = useState("准备就绪");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isTauri()) {
      setStatus("浏览器预览模式：桌面能力需在 Tauri 中使用");
      return;
    }

    invokeCommand<ModelConfig>("load_model_config")
      .then((loaded) => {
        setConfig({ ...loaded, memoryKey: loaded.memoryKey || "default" });
        return loadRecords();
      })
      .catch((err) => setError(String(err)));

    let unlistenSelection: (() => void) | undefined;
    let unlistenCleared: (() => void) | undefined;
    listen<SelectionRect>("selection-confirmed", (event) => {
      const confirmedSelection = event.payload;
      setSelection(confirmedSelection);
      setStatus("取景框已固定");
      setError(null);
    })
      .then((stop) => {
        unlistenSelection = stop;
      })
      .catch((err) => setError(String(err)));

    listen("selection-cleared", () => {
      setSelection(null);
      setStatus("选区已取消");
      setError(null);
    })
      .then((stop) => {
        unlistenCleared = stop;
      })
      .catch((err) => setError(String(err)));

    return () => {
      unlistenSelection?.();
      unlistenCleared?.();
    };
  }, []);

  useEffect(() => {
    if (!result) {
      setCorrectedScore("");
      setCorrectionNote("");
      setMemoryMessage(null);
      return;
    }

    setCorrectedScore(String(result.score));
    setCorrectionNote("");
    setMemoryMessage(null);
  }, [result]);

  async function loadRecords() {
    if (!isTauri()) return;
    try {
      const loaded = await invokeCommand<GradeRecord[]>("load_grade_records");
      setRecords(loaded);
    } catch (err) {
      setError(String(err));
    }
  }

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
          memoryKey: config.memoryKey,
          apiKey: apiKey.trim() ? apiKey.trim() : null
        }
      });
      setConfig(saved);
      setApiKey("");
      await loadRecords();
      setStatus("模型配置与记忆 Key 已保存");
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
      setStatus(selection ? "正在重新框选作文区域" : "正在框选作文区域");
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
    setMemoryMessage(null);
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
      await loadRecords();
      setStatus(mode === "auto" ? "自动阅卷完成" : "单次阅卷完成");
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(null);
    }
  }

  async function saveCorrection() {
    if (!result) return;

    const score = Number(correctedScore);
    if (!Number.isFinite(score) || score < 0 || score > maxScore) {
      setError(`订正分数必须在 0 到 ${maxScore} 之间`);
      return;
    }

    setLoading("correction");
    setError(null);
    try {
      const saved = await invokeCommand<MemoryWikiSaveResult>("save_score_correction", {
        input: {
          result,
          correctedScore: score,
          correctionNote,
          rubric,
          referenceEssay,
          maxScore
        }
      });
      setResult({ ...result, score });
      setMemoryMessage(`已写入 Memory Wiki：${saved.path}`);
      setStatus("人工订正已保存");
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(null);
    }
  }

  const selectionBoxStyle = useMemo<React.CSSProperties>(() => {
    if (!selection) return {};

    const ratio = selection.width / selection.height;
    if (ratio >= 1) {
      return {
        width: "78%",
        aspectRatio: `${selection.width} / ${selection.height}`
      };
    }

    return {
      height: "74%",
      aspectRatio: `${selection.width} / ${selection.height}`
    };
  }, [selection]);

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
            <span>Memory Key</span>
            <input
              value={config.memoryKey}
              onChange={(event) => setConfig({ ...config, memoryKey: event.target.value })}
              placeholder="default"
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
          <div className="selection-frame" data-ready={Boolean(selection)}>
            <div className="selection-frame-box" style={selectionBoxStyle}>
              <Target size={28} aria-hidden />
            </div>
            <span>{selection ? "屏幕取景框已保留" : "未选择取景框"}</span>
          </div>
          <div className="selection-meta">
            <span>{formatRect(selection)}</span>
            <span>{selection ? `缩放 ${selection.scaleFactor.toFixed(2)}` : "等待框选"}</span>
          </div>
          <div className="button-row single">
            <button onClick={openSelectionWindow} disabled={loading === "selection"}>
              <Target size={17} />
              {selection ? "重新框选" : "框选作文"}
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
              <div className="timing-strip">
                <span>总耗时 {formatMs(result.timingsMs.total)}</span>
                <span>截图 {formatMs(result.timingsMs.capture)}</span>
                <span>模型 {formatMs(result.timingsMs.api)}</span>
                <span>解析 {formatMs(result.timingsMs.parse)}</span>
              </div>
              <section className="correction-box">
                <div className="correction-title">
                  <PencilLine size={16} aria-hidden />
                  <h3>人工订正</h3>
                </div>
                <div className="correction-grid">
                  <label>
                    <span>订正分数</span>
                    <input
                      className="score-input"
                      type="number"
                      min={0}
                      max={maxScore}
                      value={correctedScore}
                      onChange={(event) => setCorrectedScore(event.target.value)}
                    />
                  </label>
                  <button onClick={saveCorrection} disabled={loading === "correction"}>
                    {loading === "correction" ? <Loader2 className="spin" size={17} /> : <Save size={17} />}
                    写入 Memory Wiki
                  </button>
                </div>
                <textarea
                  className="correction-note"
                  value={correctionNote}
                  onChange={(event) => setCorrectionNote(event.target.value)}
                  placeholder="记录你为什么改分，后续阅卷会参考这条订正规则"
                />
                {memoryMessage ? <p className="memory-message">{memoryMessage}</p> : null}
              </section>
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
          <section className="records-panel">
            <div className="records-title">
              <h3>评分记录</h3>
              <button onClick={loadRecords} disabled={loading !== null}>刷新</button>
            </div>
            {records.length > 0 ? (
              <div className="records-list">
                {records.map((record) => (
                  <article className="record-item" key={record.id} data-status={record.status}>
                    <div>
                      <strong>
                        {record.status === "success"
                          ? `${record.score?.toFixed(1) ?? "-"} / ${record.maxScore}`
                          : "失败"}
                      </strong>
                      <span>{new Date(record.timestamp * 1000).toLocaleString()}</span>
                    </div>
                    <p>{record.status === "success" ? record.comments || record.extractedText || "已记录" : record.error}</p>
                    <footer>
                      <span>{record.model}</span>
                      <span>总耗时 {formatMs(record.timingsMs?.total)}</span>
                      <span>模型 {formatMs(record.timingsMs?.api)}</span>
                    </footer>
                  </article>
                ))}
              </div>
            ) : (
              <p className="records-empty">当前 Memory Key 还没有评分记录</p>
            )}
          </section>
        </section>
      </section>
    </main>
  );
}

createRoot(document.getElementById("root")!).render(<App />);
