use std::{
    fs,
    io::Cursor,
    path::PathBuf,
    sync::Mutex,
};

use base64::{engine::general_purpose, Engine as _};
use image::{DynamicImage, ImageFormat};
use keyring::Entry;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder};
use url::Url;
use xcap::Monitor;

const SERVICE_NAME: &str = "com.paper-checker.app";
const KEYCHAIN_ACCOUNT: &str = "openai-compatible-api-key";
const CONFIG_FILE: &str = "config.json";

#[derive(Default)]
struct AppState {
    latest_selection: Mutex<Option<SelectionRect>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfig {
    base_url: String,
    model: String,
    has_api_key: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfigInput {
    base_url: String,
    model: String,
    api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredConfig {
    base_url: String,
    model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionRect {
    monitor_id: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    scale_factor: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GradeInput {
    selection: SelectionRect,
    rubric: String,
    reference_essay: String,
    max_score: f64,
    mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GradeResult {
    score: f64,
    max_score: f64,
    level: String,
    extracted_text: String,
    breakdown: Value,
    comments: String,
    suggestions: String,
    raw: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PartialGradeResult {
    score: Option<f64>,
    max_score: Option<f64>,
    level: Option<String>,
    extracted_text: Option<String>,
    breakdown: Option<Value>,
    comments: Option<String>,
    suggestions: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct PhysicalSelection {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

#[tauri::command]
async fn load_model_config(app: AppHandle) -> Result<ModelConfig, String> {
    let stored = read_stored_config(&app)?;
    Ok(ModelConfig {
        base_url: stored.base_url,
        model: stored.model,
        has_api_key: read_api_key().is_ok(),
    })
}

#[tauri::command]
async fn save_model_config(app: AppHandle, input: ModelConfigInput) -> Result<ModelConfig, String> {
    let base_url = normalize_base_url(&input.base_url)?;
    let model = input.model.trim().to_string();
    if model.is_empty() {
        return Err("模型名称不能为空".into());
    }

    if let Some(api_key) = input.api_key.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
        write_api_key(api_key)?;
    }

    let stored = StoredConfig {
        base_url,
        model,
    };
    write_stored_config(&app, &stored)?;

    Ok(ModelConfig {
        base_url: stored.base_url,
        model: stored.model,
        has_api_key: read_api_key().is_ok(),
    })
}

#[tauri::command]
async fn open_selection_window(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("selection") {
        window.set_focus().map_err(to_error)?;
        return Ok(());
    }

    WebviewWindowBuilder::new(&app, "selection", WebviewUrl::App("selection.html".into()))
        .title("框选作文区域")
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .fullscreen(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(true)
        .build()
        .map_err(to_error)?;

    Ok(())
}

#[tauri::command]
async fn confirm_selection(
    app: AppHandle,
    state: State<'_, AppState>,
    selection: SelectionRect,
) -> Result<(), String> {
    validate_selection(&selection)?;
    *state.latest_selection.lock().map_err(|_| "选区状态已被占用")? = Some(selection.clone());
    close_selection_window(&app)?;
    app.emit("selection-confirmed", selection).map_err(to_error)?;
    Ok(())
}

#[tauri::command]
async fn cancel_selection(app: AppHandle) -> Result<(), String> {
    close_selection_window(&app)?;
    Ok(())
}

#[tauri::command]
async fn capture_selection_preview(selection: SelectionRect) -> Result<String, String> {
    let png = capture_selection_png(&selection)?;
    Ok(data_url_from_png(&png))
}

#[tauri::command]
async fn grade_selection(app: AppHandle, input: GradeInput) -> Result<GradeResult, String> {
    validate_grade_input(&input)?;
    let config = read_stored_config(&app)?;
    let api_key = read_api_key().map_err(|_| "请先保存 API Key".to_string())?;
    let png = capture_selection_png(&input.selection)?;
    let image_url = data_url_from_png(&png);
    let endpoint = format!("{}/chat/completions", normalize_base_url(&config.base_url)?);
    let prompt = build_grading_prompt(&input);

    let request_body = json!({
        "model": config.model,
        "messages": [
            {
                "role": "system",
                "content": "你是一名严格、稳定、可解释的中文作文阅卷老师。你必须根据用户给出的评分标准和参考范文批改图片中的作文，并只返回 JSON。"
            },
            {
                "role": "user",
                "content": [
                    { "type": "text", "text": prompt },
                    {
                        "type": "image_url",
                        "image_url": { "url": image_url }
                    }
                ]
            }
        ],
        "temperature": 0.2,
        "max_tokens": 1800
    });

    let raw = call_chat_completions(&endpoint, &api_key, &request_body).await?;
    parse_grade_response(&raw, input.max_score)
}

fn close_selection_window(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("selection") {
        window.close().map_err(to_error)?;
    }
    Ok(())
}

fn default_stored_config() -> StoredConfig {
    StoredConfig {
        base_url: "https://api.openai.com/v1".to_string(),
        model: "gpt-4.1-mini".to_string(),
    }
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(to_error)?;
    fs::create_dir_all(&dir).map_err(to_error)?;
    Ok(dir.join(CONFIG_FILE))
}

fn read_stored_config(app: &AppHandle) -> Result<StoredConfig, String> {
    let path = config_path(app)?;
    if !path.exists() {
        return Ok(default_stored_config());
    }

    let content = fs::read_to_string(path).map_err(to_error)?;
    let mut stored: StoredConfig = serde_json::from_str(&content).map_err(to_error)?;
    stored.base_url = normalize_base_url(&stored.base_url)?;
    stored.model = stored.model.trim().to_string();
    if stored.model.is_empty() {
        stored.model = default_stored_config().model;
    }
    Ok(stored)
}

fn write_stored_config(app: &AppHandle, stored: &StoredConfig) -> Result<(), String> {
    let path = config_path(app)?;
    let content = serde_json::to_string_pretty(stored).map_err(to_error)?;
    fs::write(path, content).map_err(to_error)
}

fn keyring_entry() -> Result<Entry, String> {
    Entry::new(SERVICE_NAME, KEYCHAIN_ACCOUNT).map_err(to_error)
}

fn read_api_key() -> Result<String, String> {
    keyring_entry()?.get_password().map_err(to_error)
}

fn write_api_key(api_key: &str) -> Result<(), String> {
    keyring_entry()?.set_password(api_key).map_err(to_error)
}

fn normalize_base_url(base_url: &str) -> Result<String, String> {
    let normalized = base_url.trim().trim_end_matches('/').to_string();
    if normalized.is_empty() {
        return Err("Base URL 不能为空".into());
    }

    let parsed = Url::parse(&normalized).map_err(|_| "Base URL 格式不正确".to_string())?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("Base URL 必须使用 http 或 https".into());
    }

    Ok(normalized)
}

fn validate_selection(selection: &SelectionRect) -> Result<(), String> {
    if selection.width < 24.0 || selection.height < 24.0 {
        return Err("选区过小，请重新框选作文区域".into());
    }
    if selection.scale_factor <= 0.0 {
        return Err("选区缩放比例异常".into());
    }
    Ok(())
}

fn validate_grade_input(input: &GradeInput) -> Result<(), String> {
    validate_selection(&input.selection)?;
    if input.rubric.trim().is_empty() {
        return Err("评分标准不能为空".into());
    }
    if input.reference_essay.trim().is_empty() {
        return Err("参考范文不能为空".into());
    }
    if input.max_score <= 0.0 {
        return Err("满分必须大于 0".into());
    }
    Ok(())
}

fn rect_to_physical(selection: &SelectionRect) -> PhysicalSelection {
    let scale = selection.scale_factor.max(0.1);
    PhysicalSelection {
        x: (selection.x * scale).round() as i32,
        y: (selection.y * scale).round() as i32,
        width: (selection.width * scale).round().max(1.0) as u32,
        height: (selection.height * scale).round().max(1.0) as u32,
    }
}

fn capture_selection_png(selection: &SelectionRect) -> Result<Vec<u8>, String> {
    validate_selection(selection)?;
    let monitors = Monitor::all().map_err(map_capture_error)?;
    let monitor = choose_monitor(monitors, &selection.monitor_id)?;
    let monitor_width = monitor.width().map_err(map_capture_error)? as i32;
    let monitor_height = monitor.height().map_err(map_capture_error)? as i32;
    let physical = rect_to_physical(selection);

    let x = physical.x.clamp(0, monitor_width.saturating_sub(1));
    let y = physical.y.clamp(0, monitor_height.saturating_sub(1));
    let width = physical
        .width
        .min((monitor_width - x).max(1) as u32);
    let height = physical
        .height
        .min((monitor_height - y).max(1) as u32);

    let image = monitor
        .capture_region(x, y, width, height)
        .map_err(map_capture_error)?;

    let mut cursor = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut cursor, ImageFormat::Png)
        .map_err(to_error)?;
    Ok(cursor.into_inner())
}

fn choose_monitor(mut monitors: Vec<Monitor>, monitor_id: &str) -> Result<Monitor, String> {
    if monitors.is_empty() {
        return Err("未检测到显示器".into());
    }

    if !monitor_id.trim().is_empty() && monitor_id != "primary" {
        if let Some(index) = monitors
            .iter()
            .position(|monitor| monitor.name().unwrap_or_default() == monitor_id)
        {
            return Ok(monitors.remove(index));
        }
    }

    if let Some(index) = monitors
        .iter()
        .position(|monitor| monitor.is_primary().unwrap_or(false))
    {
        return Ok(monitors.remove(index));
    }

    Ok(monitors.remove(0))
}

fn data_url_from_png(png: &[u8]) -> String {
    format!(
        "data:image/png;base64,{}",
        general_purpose::STANDARD.encode(png)
    )
}

fn build_grading_prompt(input: &GradeInput) -> String {
    format!(
        r#"请识别图片中的作文正文，并根据评分标准与参考范文给出批改结果。

阅卷模式：{mode}
满分：{max_score}

评分标准：
{rubric}

参考范文：
{reference_essay}

只返回一个 JSON 对象，不要返回 Markdown，不要包裹代码块。字段必须使用以下英文 key：
{{
  "score": number,
  "maxScore": number,
  "level": string,
  "extractedText": string,
  "breakdown": array | object,
  "comments": string,
  "suggestions": string
}}

要求：
1. score 必须在 0 到满分之间。
2. extractedText 填入你从图片中识别出的作文正文。
3. comments 说明主要得分与扣分原因。
4. suggestions 给出可操作的改进建议。
"#,
        mode = input.mode,
        max_score = input.max_score,
        rubric = input.rubric.trim(),
        reference_essay = input.reference_essay.trim()
    )
}

async fn call_chat_completions(endpoint: &str, api_key: &str, body: &Value) -> Result<String, String> {
    let response = reqwest::Client::new()
        .post(endpoint)
        .bearer_auth(api_key)
        .json(body)
        .send()
        .await
        .map_err(map_network_error)?;

    let status = response.status();
    let text = response.text().await.map_err(map_network_error)?;
    if !status.is_success() {
        return Err(format_model_http_error(status, &text));
    }

    let value: Value = serde_json::from_str(&text)
        .map_err(|_| format!("模型接口返回了非 JSON 响应：{}", compact(&text)))?;
    extract_message_content(&value)
        .ok_or_else(|| format!("模型响应缺少 choices[0].message.content：{}", compact(&text)))
}

fn extract_message_content(value: &Value) -> Option<String> {
    let content = value.pointer("/choices/0/message/content")?;
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }

    if let Some(parts) = content.as_array() {
        let joined = parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        if !joined.is_empty() {
            return Some(joined);
        }
    }

    None
}

fn parse_grade_response(raw: &str, requested_max_score: f64) -> Result<GradeResult, String> {
    let json_text = extract_json_object(raw).unwrap_or_else(|| raw.trim().to_string());
    let parsed: PartialGradeResult = serde_json::from_str(&json_text)
        .map_err(|_| format!("模型返回格式异常：{}", compact(raw)))?;

    let max_score = parsed
        .max_score
        .filter(|value| *value > 0.0)
        .unwrap_or(requested_max_score);
    let score = clamp_score(parsed.score.unwrap_or(0.0), max_score);

    Ok(GradeResult {
        score,
        max_score,
        level: parsed.level.unwrap_or_default(),
        extracted_text: parsed.extracted_text.unwrap_or_default(),
        breakdown: parsed.breakdown.unwrap_or_else(|| json!([])),
        comments: parsed.comments.unwrap_or_default(),
        suggestions: parsed.suggestions.unwrap_or_default(),
        raw: raw.to_string(),
    })
}

fn extract_json_object(raw: &str) -> Option<String> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end <= start {
        return None;
    }
    Some(raw[start..=end].to_string())
}

fn clamp_score(score: f64, max_score: f64) -> f64 {
    if !score.is_finite() {
        return 0.0;
    }
    score.max(0.0).min(max_score)
}

fn format_model_http_error(status: StatusCode, body: &str) -> String {
    match status.as_u16() {
        400 => format!("模型接口拒绝请求，可能是不支持图片输入或请求格式不兼容：{}", compact(body)),
        401 | 403 => "模型接口鉴权失败，请检查 API Key".to_string(),
        404 => "模型接口地址不存在，请检查 Base URL 是否已经包含 /v1".to_string(),
        429 => "模型接口限流，请稍后重试".to_string(),
        _ => format!("模型接口返回 HTTP {}：{}", status.as_u16(), compact(body)),
    }
}

fn map_network_error(error: reqwest::Error) -> String {
    if error.is_timeout() {
        return "模型接口请求超时".into();
    }
    if error.is_connect() {
        return "无法连接模型接口，请检查 Base URL 和网络".into();
    }
    to_error(error)
}

fn map_capture_error<E: std::fmt::Display>(error: E) -> String {
    format!("截图失败，请确认屏幕录制/截图权限已开启：{}", error)
}

fn compact(value: &str) -> String {
    let single_line = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.chars().count() <= 240 {
        single_line
    } else {
        format!("{}...", single_line.chars().take(240).collect::<String>())
    }
}

fn to_error<E: std::fmt::Display>(error: E) -> String {
    error.to_string()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            load_model_config,
            save_model_config,
            open_selection_window,
            confirm_selection,
            cancel_selection,
            capture_selection_preview,
            grade_selection
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_base_url_without_adding_v1() {
        assert_eq!(
            normalize_base_url("https://example.com/openai/v1/").unwrap(),
            "https://example.com/openai/v1"
        );
    }

    #[test]
    fn rejects_invalid_base_url() {
        assert!(normalize_base_url("ftp://example.com").is_err());
        assert!(normalize_base_url("").is_err());
    }

    #[test]
    fn clamps_score_to_requested_range() {
        assert_eq!(clamp_score(80.0, 60.0), 60.0);
        assert_eq!(clamp_score(-1.0, 60.0), 0.0);
        assert_eq!(clamp_score(f64::NAN, 60.0), 0.0);
    }

    #[test]
    fn converts_logical_rect_to_physical_pixels() {
        let selection = SelectionRect {
            monitor_id: "primary".into(),
            x: 10.0,
            y: 20.0,
            width: 200.0,
            height: 100.0,
            scale_factor: 1.5,
        };
        let physical = rect_to_physical(&selection);
        assert_eq!(physical.x, 15);
        assert_eq!(physical.y, 30);
        assert_eq!(physical.width, 300);
        assert_eq!(physical.height, 150);
    }

    #[test]
    fn parses_json_wrapped_by_markdown() {
        let raw = r#"```json
        {"score": 72, "maxScore": 60, "level": "A", "extractedText": "正文", "comments": "好", "suggestions": "继续"}
        ```"#;
        let result = parse_grade_response(raw, 60.0).unwrap();
        assert_eq!(result.score, 60.0);
        assert_eq!(result.max_score, 60.0);
        assert_eq!(result.extracted_text, "正文");
    }

    #[test]
    fn reports_non_json_model_output() {
        assert!(parse_grade_response("不是 JSON", 60.0).is_err());
    }
}
