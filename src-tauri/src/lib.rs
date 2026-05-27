use std::{
    fs::{self, OpenOptions},
    io::{Cursor, Write},
    path::PathBuf,
    sync::Mutex,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
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
const MEMORIES_DIR: &str = "memories";
const MEMORY_WIKI_FILE: &str = "memory-wiki.md";
const GRADING_RECORDS_FILE: &str = "grading-records.jsonl";
const CAPTURES_DIR: &str = "captures";
const MEMORY_PROMPT_CHAR_LIMIT: usize = 6000;
const GRADE_RECORD_LIMIT: usize = 30;

#[derive(Default)]
struct AppState {
    latest_selection: Mutex<Option<SelectionRect>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfig {
    base_url: String,
    model: String,
    memory_key: String,
    has_api_key: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfigInput {
    base_url: String,
    model: String,
    memory_key: String,
    api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredConfig {
    base_url: String,
    model: String,
    #[serde(default = "default_memory_key")]
    memory_key: String,
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
    record_id: String,
    record_path: String,
    timings_ms: Value,
    score: f64,
    max_score: f64,
    level: String,
    extracted_text: String,
    breakdown: Value,
    comments: String,
    suggestions: String,
    raw: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrectionInput {
    result: GradeResult,
    corrected_score: f64,
    correction_note: String,
    rubric: String,
    reference_essay: String,
    max_score: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryWikiSaveResult {
    path: String,
    entry: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GradeRecord {
    id: String,
    timestamp: u64,
    memory_key: String,
    status: String,
    mode: String,
    model: String,
    base_url: String,
    max_score: f64,
    score: Option<f64>,
    level: Option<String>,
    error: Option<String>,
    timings_ms: Value,
    selection: SelectionRect,
    capture_path: Option<String>,
    record_path: Option<String>,
    extracted_text: Option<String>,
    comments: Option<String>,
    raw: Option<String>,
    rubric_excerpt: String,
    reference_essay_excerpt: String,
    prompt_excerpt: Option<String>,
    image_data_url_bytes: Option<usize>,
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
        memory_key: stored.memory_key,
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
    let memory_key = normalize_memory_key(&input.memory_key);

    if let Some(api_key) = input
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        write_api_key(api_key)?;
    }

    let stored = StoredConfig {
        base_url,
        model,
        memory_key,
    };
    write_stored_config(&app, &stored)?;

    Ok(ModelConfig {
        base_url: stored.base_url,
        model: stored.model,
        memory_key: stored.memory_key,
        has_api_key: read_api_key().is_ok(),
    })
}

#[tauri::command]
async fn open_selection_window(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("selection") {
        window.set_ignore_cursor_events(false).map_err(to_error)?;
        window.show().map_err(to_error)?;
        window.set_focus().map_err(to_error)?;
        window.emit("selection-reset", ()).map_err(to_error)?;
        return Ok(());
    }

    let (x, y, width, height) = primary_monitor_logical_bounds(&app)?;

    WebviewWindowBuilder::new(&app, "selection", WebviewUrl::App("selection.html".into()))
        .title("框选作文区域")
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .position(x, y)
        .inner_size(width, height)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
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
    *state
        .latest_selection
        .lock()
        .map_err(|_| "选区状态已被占用")? = Some(selection.clone());
    if let Some(window) = app.get_webview_window("selection") {
        window.set_ignore_cursor_events(true).map_err(to_error)?;
    }
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_focus();
    }
    app.emit("selection-confirmed", selection)
        .map_err(to_error)?;
    Ok(())
}

#[tauri::command]
async fn cancel_selection(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    *state
        .latest_selection
        .lock()
        .map_err(|_| "选区状态已被占用")? = None;
    close_selection_window(&app)?;
    app.emit("selection-cleared", ()).map_err(to_error)?;
    Ok(())
}

#[tauri::command]
async fn save_score_correction(
    app: AppHandle,
    input: CorrectionInput,
) -> Result<MemoryWikiSaveResult, String> {
    validate_correction_input(&input)?;
    let config = read_stored_config(&app)?;
    let corrected_score = clamp_score(input.corrected_score, input.max_score);
    let path = memory_wiki_path_for_key(&app, &config.memory_key)?;
    let needs_header = !path.exists()
        || fs::metadata(&path)
            .map(|meta| meta.len() == 0)
            .unwrap_or(true);
    let entry = format_memory_wiki_entry(&input, corrected_score);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(to_error)?;

    if needs_header {
        file.write_all(b"# Essay Grading Memory Wiki\n\n")
            .map_err(to_error)?;
    }
    file.write_all(entry.as_bytes()).map_err(to_error)?;

    Ok(MemoryWikiSaveResult {
        path: path.display().to_string(),
        entry,
    })
}

#[tauri::command]
async fn load_grade_records(app: AppHandle) -> Result<Vec<GradeRecord>, String> {
    let config = read_stored_config(&app)?;
    let path = grading_records_path_for_key(&app, &config.memory_key)?;
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(path).map_err(to_error)?;
    let mut records = content
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<GradeRecord>(line).ok())
        .take(GRADE_RECORD_LIMIT)
        .collect::<Vec<_>>();
    records.shrink_to_fit();
    Ok(records)
}

#[tauri::command]
async fn grade_selection(app: AppHandle, input: GradeInput) -> Result<GradeResult, String> {
    let total_start = Instant::now();
    validate_grade_input(&input)?;
    let config = read_stored_config(&app)?;
    let record_id = format!("grade-{}", unix_millis());
    let record_path = grading_records_path_for_key(&app, &config.memory_key)?;
    let mut timings = serde_json::Map::new();

    let api_key = match read_api_key() {
        Ok(value) => value,
        Err(_) => {
            timings.insert("total".into(), json!(elapsed_ms(total_start)));
            append_grade_record(
                &app,
                &config.memory_key,
                build_grade_record(
                    &record_id,
                    unix_seconds(),
                    &config,
                    &input,
                    "error",
                    Value::Object(timings.clone()),
                    None,
                    Some("请先保存 API Key".to_string()),
                    None,
                    None,
                    None,
                    None,
                    Some(record_path.display().to_string()),
                ),
            )?;
            return Err("请先保存 API Key".to_string());
        }
    };

    let capture_start = Instant::now();
    let png = match capture_selection_png_for_grading(&app, &input.selection) {
        Ok(value) => {
            timings.insert("capture".into(), json!(elapsed_ms(capture_start)));
            value
        }
        Err(error) => {
            timings.insert("capture".into(), json!(elapsed_ms(capture_start)));
            timings.insert("total".into(), json!(elapsed_ms(total_start)));
            append_grade_record(
                &app,
                &config.memory_key,
                build_grade_record(
                    &record_id,
                    unix_seconds(),
                    &config,
                    &input,
                    "error",
                    Value::Object(timings.clone()),
                    None,
                    Some(error.clone()),
                    None,
                    None,
                    None,
                    None,
                    Some(record_path.display().to_string()),
                ),
            )?;
            return Err(error);
        }
    };

    let capture_path = save_capture_png(&app, &config.memory_key, &record_id, &png)?;
    let image_url = data_url_from_png(&png);
    let endpoint = format!("{}/chat/completions", normalize_base_url(&config.base_url)?);
    let memory_start = Instant::now();
    let memory_wiki = read_memory_wiki_for_prompt(&app, &config.memory_key)?;
    timings.insert("memory".into(), json!(elapsed_ms(memory_start)));
    let prompt = build_grading_prompt(&input, &memory_wiki);

    let request_start = Instant::now();
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
    timings.insert("requestBuild".into(), json!(elapsed_ms(request_start)));

    let api_start = Instant::now();
    let raw = match call_chat_completions(&endpoint, &api_key, &request_body).await {
        Ok(value) => {
            timings.insert("api".into(), json!(elapsed_ms(api_start)));
            value
        }
        Err(error) => {
            timings.insert("api".into(), json!(elapsed_ms(api_start)));
            timings.insert("total".into(), json!(elapsed_ms(total_start)));
            append_grade_record(
                &app,
                &config.memory_key,
                build_grade_record(
                    &record_id,
                    unix_seconds(),
                    &config,
                    &input,
                    "error",
                    Value::Object(timings.clone()),
                    Some(capture_path.display().to_string()),
                    Some(error.clone()),
                    None,
                    None,
                    None,
                    None,
                    Some(record_path.display().to_string()),
                ),
            )?;
            return Err(error);
        }
    };

    let parse_start = Instant::now();
    let mut result = match parse_grade_response(&raw, input.max_score) {
        Ok(value) => {
            timings.insert("parse".into(), json!(elapsed_ms(parse_start)));
            value
        }
        Err(error) => {
            timings.insert("parse".into(), json!(elapsed_ms(parse_start)));
            timings.insert("total".into(), json!(elapsed_ms(total_start)));
            append_grade_record(
                &app,
                &config.memory_key,
                build_grade_record(
                    &record_id,
                    unix_seconds(),
                    &config,
                    &input,
                    "error",
                    Value::Object(timings.clone()),
                    Some(capture_path.display().to_string()),
                    Some(error.clone()),
                    None,
                    None,
                    None,
                    Some(raw),
                    Some(record_path.display().to_string()),
                ),
            )?;
            return Err(error);
        }
    };

    timings.insert("total".into(), json!(elapsed_ms(total_start)));
    result.record_id = record_id.clone();
    result.record_path = record_path.display().to_string();
    result.timings_ms = Value::Object(timings.clone());

    append_grade_record(
        &app,
        &config.memory_key,
        build_grade_record(
            &record_id,
            unix_seconds(),
            &config,
            &input,
            "success",
            Value::Object(timings),
            Some(capture_path.display().to_string()),
            None,
            Some(&result),
            Some(prompt.as_str()),
            Some(image_url.len()),
            Some(result.raw.clone()),
            Some(record_path.display().to_string()),
        ),
    )?;

    Ok(result)
}

fn close_selection_window(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("selection") {
        window.close().map_err(to_error)?;
    }
    Ok(())
}

fn primary_monitor_logical_bounds(app: &AppHandle) -> Result<(f64, f64, f64, f64), String> {
    let monitor = app
        .primary_monitor()
        .map_err(to_error)?
        .ok_or_else(|| "未检测到主显示器".to_string())?;
    let scale = monitor.scale_factor().max(0.1);
    let position = monitor.position();
    let size = monitor.size();

    Ok((
        position.x as f64 / scale,
        position.y as f64 / scale,
        size.width as f64 / scale,
        size.height as f64 / scale,
    ))
}

fn default_stored_config() -> StoredConfig {
    StoredConfig {
        base_url: "https://api.openai.com/v1".to_string(),
        model: "gpt-4.1-mini".to_string(),
        memory_key: "default".to_string(),
    }
}

fn default_memory_key() -> String {
    "default".to_string()
}

fn app_config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_config_dir().map_err(to_error)?;
    fs::create_dir_all(&dir).map_err(to_error)?;
    Ok(dir)
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app_config_dir(app)?.join(CONFIG_FILE))
}

fn memory_key_dir(app: &AppHandle, memory_key: &str) -> Result<PathBuf, String> {
    let dir = app_config_dir(app)?
        .join(MEMORIES_DIR)
        .join(memory_key_dir_name(memory_key));
    fs::create_dir_all(&dir).map_err(to_error)?;
    Ok(dir)
}

fn memory_wiki_path_for_key(app: &AppHandle, memory_key: &str) -> Result<PathBuf, String> {
    Ok(memory_key_dir(app, memory_key)?.join(MEMORY_WIKI_FILE))
}

fn grading_records_path_for_key(app: &AppHandle, memory_key: &str) -> Result<PathBuf, String> {
    Ok(memory_key_dir(app, memory_key)?.join(GRADING_RECORDS_FILE))
}

fn captures_dir_for_key(app: &AppHandle, memory_key: &str) -> Result<PathBuf, String> {
    let dir = memory_key_dir(app, memory_key)?.join(CAPTURES_DIR);
    fs::create_dir_all(&dir).map_err(to_error)?;
    Ok(dir)
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
    stored.memory_key = normalize_memory_key(&stored.memory_key);
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

fn normalize_memory_key(memory_key: &str) -> String {
    let trimmed = memory_key.trim();
    if trimmed.is_empty() {
        return default_memory_key();
    }

    trimmed.chars().take(80).collect()
}

fn memory_key_dir_name(memory_key: &str) -> String {
    let normalized = normalize_memory_key(memory_key);
    let sanitized = normalized
        .chars()
        .map(|ch| match ch {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect::<String>()
        .trim_matches('.')
        .trim()
        .to_string();

    if sanitized.is_empty() {
        default_memory_key()
    } else {
        sanitized
    }
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

fn validate_correction_input(input: &CorrectionInput) -> Result<(), String> {
    if input.max_score <= 0.0 {
        return Err("满分必须大于 0".into());
    }
    if !input.corrected_score.is_finite() {
        return Err("订正分数格式不正确".into());
    }
    if input.corrected_score < 0.0 || input.corrected_score > input.max_score {
        return Err(format!("订正分数必须在 0 到 {} 之间", input.max_score));
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
    let width = physical.width.min((monitor_width - x).max(1) as u32);
    let height = physical.height.min((monitor_height - y).max(1) as u32);

    let image = monitor
        .capture_region(x as u32, y as u32, width, height)
        .map_err(map_capture_error)?;

    let mut cursor = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut cursor, ImageFormat::Png)
        .map_err(to_error)?;
    Ok(cursor.into_inner())
}

fn capture_selection_png_for_grading(
    app: &AppHandle,
    selection: &SelectionRect,
) -> Result<Vec<u8>, String> {
    let marker = app.get_webview_window("selection");
    if let Some(window) = marker.as_ref() {
        let _ = window.hide();
        thread::sleep(Duration::from_millis(80));
    }

    let captured = capture_selection_png(selection);

    if let Some(window) = marker.as_ref() {
        let _ = window.show();
        let _ = window.set_ignore_cursor_events(true);
    }

    captured
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

fn save_capture_png(
    app: &AppHandle,
    memory_key: &str,
    record_id: &str,
    png: &[u8],
) -> Result<PathBuf, String> {
    let path = captures_dir_for_key(app, memory_key)?.join(format!("{record_id}.png"));
    fs::write(&path, png).map_err(to_error)?;
    Ok(path)
}

fn append_grade_record(
    app: &AppHandle,
    memory_key: &str,
    record: GradeRecord,
) -> Result<(), String> {
    let path = grading_records_path_for_key(app, memory_key)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(to_error)?;
    let line = serde_json::to_string(&record).map_err(to_error)?;
    writeln!(file, "{line}").map_err(to_error)
}

#[allow(clippy::too_many_arguments)]
fn build_grade_record(
    record_id: &str,
    timestamp: u64,
    config: &StoredConfig,
    input: &GradeInput,
    status: &str,
    timings_ms: Value,
    capture_path: Option<String>,
    error: Option<String>,
    result: Option<&GradeResult>,
    prompt: Option<&str>,
    image_data_url_bytes: Option<usize>,
    raw: Option<String>,
    record_path: Option<String>,
) -> GradeRecord {
    GradeRecord {
        id: record_id.to_string(),
        timestamp,
        memory_key: config.memory_key.clone(),
        status: status.to_string(),
        mode: input.mode.clone(),
        model: config.model.clone(),
        base_url: config.base_url.clone(),
        max_score: input.max_score,
        score: result.map(|value| value.score),
        level: result
            .map(|value| value.level.clone())
            .filter(|value| !value.is_empty()),
        error,
        timings_ms,
        selection: input.selection.clone(),
        capture_path,
        record_path,
        extracted_text: result
            .map(|value| value.extracted_text.clone())
            .filter(|value| !value.is_empty()),
        comments: result
            .map(|value| value.comments.clone())
            .filter(|value| !value.is_empty()),
        raw: raw.or_else(|| result.map(|value| value.raw.clone())),
        rubric_excerpt: memory_excerpt(&input.rubric, 1800),
        reference_essay_excerpt: memory_excerpt(&input.reference_essay, 1800),
        prompt_excerpt: prompt.map(|value| memory_excerpt(value, 2400)),
        image_data_url_bytes,
    }
}

fn read_memory_wiki_for_prompt(app: &AppHandle, memory_key: &str) -> Result<String, String> {
    let path = memory_wiki_path_for_key(app, memory_key)?;
    if !path.exists() {
        return Ok(String::new());
    }

    let content = fs::read_to_string(path).map_err(to_error)?;
    Ok(tail_chars(&content, MEMORY_PROMPT_CHAR_LIMIT))
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn elapsed_ms(start: Instant) -> u64 {
    start.elapsed().as_millis() as u64
}

fn format_memory_wiki_entry(input: &CorrectionInput, corrected_score: f64) -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let delta = corrected_score - input.result.score;
    let note = if input.correction_note.trim().is_empty() {
        "未填写"
    } else {
        input.correction_note.trim()
    };

    format!(
        r#"## Correction {timestamp}

- Max score: {max_score}
- Model score: {model_score}
- Corrected score: {corrected_score}
- Delta: {delta}
- Correction note: {note}
- Rubric excerpt: {rubric}
- Reference essay excerpt: {reference_essay}
- Extracted essay excerpt: {extracted_text}
- Model comments excerpt: {comments}

"#,
        timestamp = timestamp,
        max_score = input.max_score,
        model_score = input.result.score,
        corrected_score = corrected_score,
        delta = format!("{delta:.2}"),
        note = memory_excerpt(note, 900),
        rubric = memory_excerpt(&input.rubric, 1200),
        reference_essay = memory_excerpt(&input.reference_essay, 1200),
        extracted_text = memory_excerpt(&input.result.extracted_text, 1600),
        comments = memory_excerpt(&input.result.comments, 900),
    )
}

fn memory_excerpt(value: &str, max_chars: usize) -> String {
    let normalized = value
        .trim()
        .replace('\r', "")
        .split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" / ");

    if normalized.chars().count() <= max_chars {
        normalized
    } else {
        format!(
            "{}...",
            normalized.chars().take(max_chars).collect::<String>()
        )
    }
}

fn tail_chars(value: &str, max_chars: usize) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return value.to_string();
    }

    chars[chars.len() - max_chars..].iter().collect()
}

fn build_grading_prompt(input: &GradeInput, memory_wiki: &str) -> String {
    let memory_section = if memory_wiki.trim().is_empty() {
        "暂无人工订正记忆。".to_string()
    } else {
        format!(
            "以下是用户过去手动订正形成的 Memory Wiki。它反映用户对分数松紧、扣分偏好和特殊题型的校准；如果它与当前评分标准冲突，以当前评分标准为准。\n{}",
            memory_wiki.trim()
        )
    };

    format!(
        r#"请识别图片中的作文正文，并根据评分标准与参考范文给出批改结果。

阅卷模式：{mode}
满分：{max_score}

评分标准：
{rubric}

参考范文：
{reference_essay}

人工订正 Memory Wiki：
{memory_section}

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
        reference_essay = input.reference_essay.trim(),
        memory_section = memory_section
    )
}

async fn call_chat_completions(
    endpoint: &str,
    api_key: &str,
    body: &Value,
) -> Result<String, String> {
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
    extract_message_content(&value).ok_or_else(|| {
        format!(
            "模型响应缺少 choices[0].message.content：{}",
            compact(&text)
        )
    })
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
        record_id: String::new(),
        record_path: String::new(),
        timings_ms: json!({}),
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
        400 => format!(
            "模型接口拒绝请求，可能是不支持图片输入或请求格式不兼容：{}",
            compact(body)
        ),
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
            save_score_correction,
            load_grade_records,
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

    #[test]
    fn memory_wiki_entry_records_correction() {
        let input = CorrectionInput {
            result: GradeResult {
                record_id: "test-record".into(),
                record_path: String::new(),
                timings_ms: json!({}),
                score: 52.0,
                max_score: 60.0,
                level: "A".into(),
                extracted_text: "这是一篇作文。".into(),
                breakdown: json!([]),
                comments: "模型认为结构完整。".into(),
                suggestions: "继续保持。".into(),
                raw: "{}".into(),
            },
            corrected_score: 46.0,
            correction_note: "跑题较明显，需要更严格扣分。".into(),
            rubric: "立意、内容、结构、语言。".into(),
            reference_essay: "参考范文。".into(),
            max_score: 60.0,
        };

        let entry = format_memory_wiki_entry(&input, 46.0);
        assert!(entry.contains("Model score: 52"));
        assert!(entry.contains("Corrected score: 46"));
        assert!(entry.contains("跑题较明显"));
    }

    #[test]
    fn grading_prompt_includes_memory_wiki() {
        let input = GradeInput {
            selection: SelectionRect {
                monitor_id: "primary".into(),
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
                scale_factor: 1.0,
            },
            rubric: "评分标准".into(),
            reference_essay: "参考范文".into(),
            max_score: 60.0,
            mode: "single".into(),
        };

        let prompt = build_grading_prompt(&input, "历史订正：跑题扣 8 分");
        assert!(prompt.contains("Memory Wiki"));
        assert!(prompt.contains("跑题扣 8 分"));
    }

    #[test]
    #[ignore = "requires an interactive desktop session with screen capture permission"]
    fn captures_primary_monitor_region_as_png() {
        let selection = SelectionRect {
            monitor_id: "primary".into(),
            x: 0.0,
            y: 0.0,
            width: 128.0,
            height: 128.0,
            scale_factor: 1.0,
        };

        let png = capture_selection_png(&selection).expect("capture primary monitor region");
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(png.len() > 128);
    }
}
