//! Batch import of a dataset source from CSV or JSON, and export back out, per
//! `docs/NUTRITION_SOURCES.md` §6/§7.1.
//!
//! `parse` is pure (bytes in, rows and issues out); `preview` checks against the current DB
//! without writing; `commit` writes in one transaction. This split lets the CLI and the desktop
//! share one code path for preview and commit.

use std::collections::HashSet;

use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

use lifeagent_core::plugin::{PluginContext, PluginError};

use crate::foods::{normalize, validate_per_100g};
use crate::model::Macros;
use crate::sources::{self, DatasetOrigin, SourceInfo};

/// File size limit (§6.2).
const MAX_BYTES: usize = 20 * 1024 * 1024;
/// Row count limit (§6.2).
const MAX_ROWS: usize = 50_000;
/// Issue lists are capped at this many entries in `ParsedImport`/`ImportPreview` (§6.2).
const MAX_ISSUES: usize = 200;
const MAX_ALIASES_PER_ROW: usize = 20;

#[derive(Debug, thiserror::Error)]
pub enum FoodDataError {
    /// User-caused: bad file, limits, zero valid rows, rename length, …
    #[error("{0}")]
    Invalid(String),
    /// Unknown source id, expired import token.
    #[error("{0}")]
    NotFound(String),
    #[error(transparent)]
    Plugin(#[from] PluginError),
}

impl From<sqlx::Error> for FoodDataError {
    fn from(e: sqlx::Error) -> Self {
        FoodDataError::Plugin(PluginError::from(e))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportFormat {
    Csv,
    Json,
}

impl ImportFormat {
    pub fn db_str(&self) -> &'static str {
        match self {
            ImportFormat::Csv => "csv",
            ImportFormat::Json => "json",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RowIssue {
    pub line: usize,
    pub name: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone)]
struct ValidRow {
    line: usize,
    name: String,
    name_key: String,
    macros: Macros,
    aliases: Vec<String>,
}

/// The result of `parse`: bytes in, rows and issues out. Fully unit tested with no DB, clock or
/// model involved.
#[derive(Debug, Clone)]
pub struct ParsedImport {
    pub format: ImportFormat,
    file_name: String,
    pub suggested_name: String,
    pub total_rows: usize,
    rows: Vec<ValidRow>,
    pub errors: Vec<RowIssue>,
    pub error_count: usize,
    pub warnings: Vec<RowIssue>,
    pub warning_count: usize,
}

impl ParsedImport {
    pub fn valid_rows(&self) -> usize {
        self.rows.len()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportRowView {
    pub name: String,
    pub kcal: f64,
    pub protein_g: f64,
    pub carbs_g: f64,
    pub fat_g: f64,
    pub fiber_g: f64,
    pub aliases: Vec<String>,
}

impl From<&ValidRow> for ImportRowView {
    fn from(row: &ValidRow) -> Self {
        let r = row.macros.rounded();
        ImportRowView {
            name: row.name.clone(),
            kcal: r.kcal,
            protein_g: r.protein_g,
            carbs_g: r.carbs_g,
            fat_g: r.fat_g,
            fiber_g: r.fiber_g,
            aliases: row.aliases.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportPreview {
    pub format: ImportFormat,
    pub suggested_name: String,
    pub total_rows: usize,
    pub valid_rows: usize,
    pub errors: Vec<RowIssue>,
    pub error_count: usize,
    pub warnings: Vec<RowIssue>,
    pub warning_count: usize,
    /// Valid rows whose name the user has set by hand — they won't take effect while `manual`
    /// has them.
    pub shadowed_by_manual: usize,
    /// Valid rows that will override an earlier AI estimate.
    pub replaces_estimates: usize,
    /// First 20 valid rows, values per 100 g, rounded for display.
    pub sample: Vec<ImportRowView>,
}

pub enum ImportTarget {
    /// Appended as the lowest-priority dataset (just above `llm_estimate`).
    New { name: String },
    /// Keeps id, name and priority; swaps all rows atomically.
    Replace { source_id: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportReport {
    pub source: SourceInfo,
    pub imported: usize,
    pub skipped: usize,
}

// ---- Cell parsing (§6.1/§6.2) -----------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum CellValue {
    Number(f64),
    /// `Tr` / `微量`.
    Trace,
    /// Empty, or one of `— – - … ... N/A NA`.
    NotMeasured,
    /// Anything else non-numeric: always a row error, in any column.
    Text,
}

fn to_halfwidth(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '０'..='９' => char::from_u32(c as u32 - '０' as u32 + '0' as u32).unwrap_or(c),
            '．' => '.',
            other => other,
        })
        .collect()
}

fn parse_cell(raw: &str) -> CellValue {
    let s = raw.trim();
    if s.is_empty() {
        return CellValue::NotMeasured;
    }
    let lower = s.to_lowercase();
    if lower == "tr" || s == "微量" {
        return CellValue::Trace;
    }
    if matches!(s, "—" | "–" | "-" | "…" | "...") || lower == "n/a" || lower == "na" {
        return CellValue::NotMeasured;
    }
    match to_halfwidth(s).parse::<f64>() {
        Ok(n) if n.is_finite() => CellValue::Number(n),
        _ => CellValue::Text,
    }
}

fn cell_from_json(v: Option<&Value>) -> CellValue {
    match v {
        None | Some(Value::Null) => CellValue::NotMeasured,
        Some(Value::Number(n)) => n.as_f64().map(CellValue::Number).unwrap_or(CellValue::Text),
        Some(Value::String(s)) => parse_cell(s),
        Some(_) => CellValue::Text,
    }
}

/// `Ok((Some(v), was_trace))` for a usable value, `Ok((None, false))` when absent/not measured,
/// `Err(())` for unrecognized text (always an error, in any column).
fn resolve_numeric(cell: &CellValue) -> Result<(Option<f64>, bool), ()> {
    match cell {
        CellValue::Number(n) => Ok((Some(*n), false)),
        CellValue::Trace => Ok((Some(0.0), true)),
        CellValue::NotMeasured => Ok((None, false)),
        CellValue::Text => Err(()),
    }
}

// ---- Column mapping (§6.1) ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Column {
    Name,
    Kcal,
    Kj,
    Protein,
    Carbs,
    Fat,
    Fiber,
    Basis,
    Aliases,
}

/// Case-insensitive, trimmed, ignoring ASCII/full-width parentheses.
fn normalize_header(h: &str) -> String {
    h.trim()
        .to_lowercase()
        .chars()
        .filter(|c| !matches!(c, '(' | ')' | '（' | '）'))
        .collect()
}

fn column_for_header(header: &str) -> Option<Column> {
    const TABLE: &[(&str, Column)] = &[
        ("name", Column::Name),
        ("名称", Column::Name),
        ("食物", Column::Name),
        ("kcal", Column::Kcal),
        ("energy_kcal", Column::Kcal),
        ("能量", Column::Kcal),
        ("热量", Column::Kcal),
        ("能量kcal", Column::Kcal),
        ("kj", Column::Kj),
        ("energy_kj", Column::Kj),
        ("能量kj", Column::Kj),
        ("protein_g", Column::Protein),
        ("protein", Column::Protein),
        ("蛋白质", Column::Protein),
        ("carbs_g", Column::Carbs),
        ("carbohydrate", Column::Carbs),
        ("碳水化合物", Column::Carbs),
        ("碳水", Column::Carbs),
        ("fat_g", Column::Fat),
        ("fat", Column::Fat),
        ("脂肪", Column::Fat),
        ("fiber_g", Column::Fiber),
        ("fiber", Column::Fiber),
        ("膳食纤维", Column::Fiber),
        ("纤维", Column::Fiber),
        ("basis_g", Column::Basis),
        ("per_g", Column::Basis),
        ("每份克数", Column::Basis),
        ("aliases", Column::Aliases),
        ("别名", Column::Aliases),
    ];
    let h = normalize_header(header);
    TABLE.iter().find(|(k, _)| *k == h).map(|(_, c)| *c)
}

fn split_aliases(field: &str) -> Vec<String> {
    field
        .split(['|', '｜'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

#[derive(Debug, Clone)]
struct RawRow {
    line: usize,
    name: Option<String>,
    kcal: CellValue,
    kj: CellValue,
    protein: CellValue,
    carbs: CellValue,
    fat: CellValue,
    fiber: CellValue,
    basis: CellValue,
    aliases: Vec<String>,
}

impl RawRow {
    fn empty(line: usize) -> Self {
        RawRow {
            line,
            name: None,
            kcal: CellValue::NotMeasured,
            kj: CellValue::NotMeasured,
            protein: CellValue::NotMeasured,
            carbs: CellValue::NotMeasured,
            fat: CellValue::NotMeasured,
            fiber: CellValue::NotMeasured,
            basis: CellValue::NotMeasured,
            aliases: Vec::new(),
        }
    }

    fn set_csv_field(&mut self, col: Column, field: &str) {
        match col {
            Column::Name => self.name = Some(field.to_string()),
            Column::Kcal => self.kcal = parse_cell(field),
            Column::Kj => self.kj = parse_cell(field),
            Column::Protein => self.protein = parse_cell(field),
            Column::Carbs => self.carbs = parse_cell(field),
            Column::Fat => self.fat = parse_cell(field),
            Column::Fiber => self.fiber = parse_cell(field),
            Column::Basis => self.basis = parse_cell(field),
            Column::Aliases => self.aliases = split_aliases(field),
        }
    }
}

// ---- Format detection and file-level parsing (§6.1) --------------------------------------------

fn detect_format(file_name: &str, text: &str) -> ImportFormat {
    let lower = file_name.to_lowercase();
    if lower.ends_with(".json") {
        return ImportFormat::Json;
    }
    if lower.ends_with(".csv") {
        return ImportFormat::Csv;
    }
    match text.trim_start().chars().next() {
        Some('{') | Some('[') => ImportFormat::Json,
        _ => ImportFormat::Csv,
    }
}

fn file_stem(file_name: &str) -> String {
    let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    match base.rfind('.') {
        Some(i) if i > 0 => base[..i].to_string(),
        _ => base.to_string(),
    }
}

fn parse_csv(text: &str) -> Result<(Vec<RawRow>, Vec<String>), FoodDataError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(text.as_bytes());
    let headers = reader
        .headers()
        .map_err(|e| FoodDataError::Invalid(format!("CSV 表头解析失败：{e}")))?
        .clone();
    let mut columns: Vec<Option<Column>> = Vec::with_capacity(headers.len());
    let mut unknown = Vec::new();
    for h in headers.iter() {
        match column_for_header(h) {
            Some(c) => columns.push(Some(c)),
            None => {
                unknown.push(h.to_string());
                columns.push(None);
            }
        }
    }

    let mut rows = Vec::new();
    for (i, record) in reader.records().enumerate() {
        let record = record
            .map_err(|e| FoodDataError::Invalid(format!("CSV 第 {} 行解析失败：{e}", i + 2)))?;
        let mut row = RawRow::empty(i + 2);
        for (idx, field) in record.iter().enumerate() {
            if let Some(Some(col)) = columns.get(idx) {
                row.set_csv_field(*col, field);
            }
        }
        rows.push(row);
    }
    Ok((rows, unknown))
}

const JSON_KNOWN_KEYS: &[&str] = &[
    "name",
    "kcal",
    "kj",
    "protein_g",
    "carbs_g",
    "fat_g",
    "fiber_g",
    "basis_g",
    "aliases",
];

/// (envelope `name`, parsed rows, unknown top-level keys seen across the file).
type ParsedJsonRows = (Option<String>, Vec<RawRow>, Vec<String>);

fn parse_json(text: &str) -> Result<ParsedJsonRows, FoodDataError> {
    let value: Value = serde_json::from_str(text)
        .map_err(|e| FoodDataError::Invalid(format!("JSON 解析失败：{e}")))?;
    let (envelope_name, array) = match value {
        Value::Array(a) => (None, a),
        Value::Object(mut obj) => {
            let name = obj
                .remove("name")
                .and_then(|v| v.as_str().map(str::to_string));
            let foods = obj
                .remove("foods")
                .ok_or_else(|| FoodDataError::Invalid("JSON 对象缺少 foods 字段".into()))?;
            let Value::Array(a) = foods else {
                return Err(FoodDataError::Invalid("foods 必须是数组".into()));
            };
            (name, a)
        }
        _ => {
            return Err(FoodDataError::Invalid(
                "JSON 必须是数组，或包含 foods 字段的对象".into(),
            ))
        }
    };

    let mut unknown = HashSet::new();
    let mut rows = Vec::with_capacity(array.len());
    for (i, item) in array.into_iter().enumerate() {
        let line = i + 1;
        let Value::Object(obj) = item else {
            return Err(FoodDataError::Invalid(format!("第 {line} 项不是对象")));
        };
        for key in obj.keys() {
            if !JSON_KNOWN_KEYS.contains(&key.as_str()) {
                unknown.insert(key.clone());
            }
        }
        let mut row = RawRow::empty(line);
        row.name = obj.get("name").and_then(|v| v.as_str()).map(str::to_string);
        row.kcal = cell_from_json(obj.get("kcal"));
        row.kj = cell_from_json(obj.get("kj"));
        row.protein = cell_from_json(obj.get("protein_g"));
        row.carbs = cell_from_json(obj.get("carbs_g"));
        row.fat = cell_from_json(obj.get("fat_g"));
        row.fiber = cell_from_json(obj.get("fiber_g"));
        row.basis = cell_from_json(obj.get("basis_g"));
        row.aliases = obj
            .get("aliases")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        rows.push(row);
    }
    Ok((envelope_name, rows, unknown.into_iter().collect()))
}

// ---- Row-level validation (§6.2) ----------------------------------------------------------------

fn process_row(raw: RawRow, trace_rows: &mut usize) -> Result<(ValidRow, Vec<RowIssue>), RowIssue> {
    let line = raw.line;
    let name = raw.name.clone().unwrap_or_default().trim().to_string();
    let issue = |message: String| RowIssue {
        line,
        name: (!name.is_empty()).then(|| name.clone()),
        message,
    };
    let mut warnings = Vec::new();

    let (basis, basis_trace) =
        resolve_numeric(&raw.basis).map_err(|_| issue("basis_g 不是数字".into()))?;
    let basis = basis.unwrap_or(100.0);
    if basis_trace {
        *trace_rows += 1;
    }
    if basis <= 0.0 {
        return Err(issue(format!("basis_g 必须大于 0，实际为 {basis}")));
    }

    let (protein, p_trace) =
        resolve_numeric(&raw.protein).map_err(|_| issue("protein_g 不是数字".into()))?;
    let protein = protein.ok_or_else(|| issue("缺少必填字段 protein_g".into()))?;
    let (carbs, c_trace) =
        resolve_numeric(&raw.carbs).map_err(|_| issue("carbs_g 不是数字".into()))?;
    let carbs = carbs.ok_or_else(|| issue("缺少必填字段 carbs_g".into()))?;
    let (fat, f_trace) = resolve_numeric(&raw.fat).map_err(|_| issue("fat_g 不是数字".into()))?;
    let fat = fat.ok_or_else(|| issue("缺少必填字段 fat_g".into()))?;
    let (fiber, fi_trace) =
        resolve_numeric(&raw.fiber).map_err(|_| issue("fiber_g 不是数字".into()))?;
    let fiber = fiber.unwrap_or(0.0);

    let (kcal_val, k_trace) =
        resolve_numeric(&raw.kcal).map_err(|_| issue("kcal 不是数字".into()))?;
    let (kj_val, kj_trace) = resolve_numeric(&raw.kj).map_err(|_| issue("kj 不是数字".into()))?;
    let kcal_from_kj = kj_val.map(|kj| kj / 4.184);
    let kcal = match (kcal_val, kcal_from_kj) {
        (Some(k), Some(kj_k)) => {
            if (k - kj_k).abs() > 0.02 * k.abs().max(1.0) {
                warnings.push(issue(format!(
                    "kcal（{k:.0}）与 kj 换算得到的 {kj_k:.0} 相差超过 2%，以 kcal 为准"
                )));
            }
            k
        }
        (Some(k), None) => k,
        (None, Some(kj_k)) => kj_k,
        (None, None) => return Err(issue("缺少必填字段 kcal 或 kj".into())),
    };
    for traced in [p_trace, c_trace, f_trace, fi_trace, k_trace, kj_trace] {
        if traced {
            *trace_rows += 1;
        }
    }

    let scale = 100.0 / basis;
    let macros = Macros {
        kcal: kcal * scale,
        protein_g: protein * scale,
        carbs_g: carbs * scale,
        fat_g: fat * scale,
        fiber_g: fiber * scale,
    };
    validate_per_100g(&macros).map_err(issue)?;

    let name_key = normalize(&name);
    if name_key.is_empty() {
        return Err(issue("食物名称不能为空".into()));
    }
    if name.chars().count() > 100 {
        return Err(issue("食物名称超过 100 字符".into()));
    }

    let e = 4.0 * macros.protein_g
        + 4.0 * (macros.carbs_g - macros.fiber_g)
        + 9.0 * macros.fat_g
        + 2.0 * macros.fiber_g;
    if (macros.kcal - e).abs() > f64::max(20.0, 0.15 * e) {
        warnings.push(issue(format!(
            "kcal（{:.0}）与营养素推算的能量（{e:.0}）相差较大，导入的数值保持不变",
            macros.kcal
        )));
    }

    let mut aliases = Vec::new();
    for alias_raw in raw.aliases.iter().take(MAX_ALIASES_PER_ROW) {
        let key = normalize(alias_raw);
        if key.is_empty() || key == name_key {
            continue;
        }
        aliases.push(key);
    }
    if raw.aliases.len() > MAX_ALIASES_PER_ROW {
        warnings.push(issue(format!(
            "别名数超过 {MAX_ALIASES_PER_ROW} 个，多余的已忽略"
        )));
    }

    Ok((
        ValidRow {
            line,
            name,
            name_key,
            macros,
            aliases,
        },
        warnings,
    ))
}

/// Pure: bytes in, rows and issues out. No DB, no clock, no model.
pub fn parse(file_name: &str, bytes: &[u8]) -> Result<ParsedImport, FoodDataError> {
    if bytes.len() > MAX_BYTES {
        return Err(FoodDataError::Invalid(format!(
            "文件超过 {} MB 限制",
            MAX_BYTES / (1024 * 1024)
        )));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| FoodDataError::Invalid("文件不是 UTF-8 编码，请用 UTF-8 另存后重试".into()))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    let format = detect_format(file_name, text);
    let (envelope_name, raw_rows, unknown_columns) = match format {
        ImportFormat::Csv => {
            let (rows, unknown) = parse_csv(text)?;
            (None, rows, unknown)
        }
        ImportFormat::Json => parse_json(text)?,
    };
    if raw_rows.len() > MAX_ROWS {
        return Err(FoodDataError::Invalid(format!(
            "文件行数超过 {MAX_ROWS} 行限制"
        )));
    }
    let suggested_name = envelope_name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| file_stem(file_name));

    let total_rows = raw_rows.len();
    let mut trace_rows = 0usize;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut candidates = Vec::new();
    let mut seen_name_keys = HashSet::new();
    for raw in raw_rows {
        match process_row(raw, &mut trace_rows) {
            Ok((row, row_warnings)) => {
                if !seen_name_keys.insert(row.name_key.clone()) {
                    errors.push(RowIssue {
                        line: row.line,
                        name: Some(row.name.clone()),
                        message: "与文件中更早一行的食物名称重复".into(),
                    });
                    continue;
                }
                warnings.extend(row_warnings);
                candidates.push(row);
            }
            Err(e) => errors.push(e),
        }
    }
    if trace_rows > 0 {
        warnings.push(RowIssue {
            line: 0,
            name: None,
            message: format!("{trace_rows} 行使用了“Tr/微量”标记，已按 0 处理"),
        });
    }
    if !unknown_columns.is_empty() {
        let mut cols = unknown_columns;
        cols.sort();
        warnings.push(RowIssue {
            line: 0,
            name: None,
            message: format!("忽略了未知列：{}", cols.join("、")),
        });
    }

    // Cross-row alias de-duplication: an alias equal to another row's name or alias in the same
    // file is dropped (§6.2). `claimed` starts with every kept row's own name, so an alias can
    // never shadow a real food name in the same file.
    let mut claimed: HashSet<String> = candidates.iter().map(|r| r.name_key.clone()).collect();
    let mut rows = Vec::with_capacity(candidates.len());
    for mut row in candidates {
        let mut kept = Vec::new();
        for alias in row.aliases.drain(..) {
            if claimed.contains(&alias) {
                warnings.push(RowIssue {
                    line: row.line,
                    name: Some(row.name.clone()),
                    message: format!("别名“{alias}”与文件中其他食物名称或别名重复，已忽略"),
                });
                continue;
            }
            claimed.insert(alias.clone());
            kept.push(alias);
        }
        row.aliases = kept;
        rows.push(row);
    }

    if rows.is_empty() {
        return Err(FoodDataError::Invalid(
            "文件中没有可导入的有效数据行".into(),
        ));
    }

    let error_count = errors.len();
    let warning_count = warnings.len();
    errors.truncate(MAX_ISSUES);
    warnings.truncate(MAX_ISSUES);

    Ok(ParsedImport {
        format,
        file_name: file_name.to_string(),
        suggested_name,
        total_rows,
        rows,
        errors,
        error_count,
        warnings,
        warning_count,
    })
}

/// Checks against the current DB (shadowing counts) without writing.
pub async fn preview(
    ctx: &PluginContext,
    parsed: &ParsedImport,
) -> Result<ImportPreview, FoodDataError> {
    let mut shadowed_by_manual = 0usize;
    let mut replaces_estimates = 0usize;
    for row in &parsed.rows {
        let manual: Option<(String,)> =
            sqlx::query_as("SELECT id FROM diet_foods WHERE source_id = 'manual' AND name_key = ?")
                .bind(&row.name_key)
                .fetch_optional(&ctx.db)
                .await?;
        if manual.is_some() {
            shadowed_by_manual += 1;
        }
        let estimate: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM diet_foods WHERE source_id = 'llm_estimate' AND name_key = ?",
        )
        .bind(&row.name_key)
        .fetch_optional(&ctx.db)
        .await?;
        if estimate.is_some() {
            replaces_estimates += 1;
        }
    }
    Ok(ImportPreview {
        format: parsed.format,
        suggested_name: parsed.suggested_name.clone(),
        total_rows: parsed.total_rows,
        valid_rows: parsed.rows.len(),
        errors: parsed.errors.clone(),
        error_count: parsed.error_count,
        warnings: parsed.warnings.clone(),
        warning_count: parsed.warning_count,
        shadowed_by_manual,
        replaces_estimates,
        sample: parsed
            .rows
            .iter()
            .take(20)
            .map(ImportRowView::from)
            .collect(),
    })
}

/// Writes in one transaction: a new dataset source, or replaces an existing dataset's rows.
pub async fn commit(
    ctx: &PluginContext,
    parsed: ParsedImport,
    target: ImportTarget,
) -> Result<ImportReport, FoodDataError> {
    let now = ctx.clock.now().to_rfc3339();
    let origin_json = serde_json::to_string(&DatasetOrigin {
        file_name: parsed.file_name.clone(),
        format: parsed.format.db_str().to_string(),
        imported_at: now.clone(),
        rows: parsed.rows.len() as i64,
    })
    .expect("DatasetOrigin always serializes");

    let mut tx = ctx.db.begin().await?;
    let mut carried_counts: std::collections::HashMap<String, i64> =
        std::collections::HashMap::new();

    let source_id = match &target {
        ImportTarget::New { name } => {
            let name = name.trim();
            if name.is_empty() || name.chars().count() > 60 {
                return Err(FoodDataError::Invalid("数据源名称需为 1-60 个字符".into()));
            }
            let (max_priority,): (i64,) = sqlx::query_as(
                "SELECT COALESCE(MAX(priority), 0) FROM diet_food_sources WHERE kind = 'dataset'",
            )
            .fetch_one(&mut *tx)
            .await?;
            let id = Uuid::now_v7().to_string();
            sqlx::query(
                "INSERT INTO diet_food_sources
                    (id, kind, name, priority, enabled, origin_json, created_at, updated_at)
                 VALUES (?, 'dataset', ?, ?, 1, ?, ?, ?)",
            )
            .bind(&id)
            .bind(name)
            .bind(max_priority + 1)
            .bind(&origin_json)
            .bind(&now)
            .bind(&now)
            .execute(&mut *tx)
            .await?;
            id
        }
        ImportTarget::Replace { source_id } => {
            let kind: Option<(String,)> =
                sqlx::query_as("SELECT kind FROM diet_food_sources WHERE id = ?")
                    .bind(source_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            match kind {
                Some((k,)) if k == "dataset" => {}
                Some(_) => return Err(FoodDataError::Invalid("只能替换数据集来源".into())),
                None => {
                    return Err(FoodDataError::NotFound(format!(
                        "数据源不存在：{source_id}"
                    )))
                }
            }
            let old: Vec<(String, i64)> =
                sqlx::query_as("SELECT name_key, use_count FROM diet_foods WHERE source_id = ?")
                    .bind(source_id)
                    .fetch_all(&mut *tx)
                    .await?;
            carried_counts = old.into_iter().collect();
            sqlx::query("DELETE FROM diet_foods WHERE source_id = ?")
                .bind(source_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "UPDATE diet_food_sources SET origin_json = ?, updated_at = ? WHERE id = ?",
            )
            .bind(&origin_json)
            .bind(&now)
            .bind(source_id)
            .execute(&mut *tx)
            .await?;
            source_id.clone()
        }
    };

    let mut imported = 0usize;
    for row in &parsed.rows {
        let id = Uuid::now_v7().to_string();
        let use_count = carried_counts.get(&row.name_key).copied().unwrap_or(0);
        sqlx::query(
            "INSERT INTO diet_foods
                (id, source_id, name, name_key, kcal, protein_g, carbs_g, fat_g, fiber_g,
                 use_count, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&source_id)
        .bind(&row.name)
        .bind(&row.name_key)
        .bind(row.macros.kcal)
        .bind(row.macros.protein_g)
        .bind(row.macros.carbs_g)
        .bind(row.macros.fat_g)
        .bind(row.macros.fiber_g)
        .bind(use_count)
        .bind(&now)
        .bind(&now)
        .execute(&mut *tx)
        .await?;
        for alias in &row.aliases {
            sqlx::query(
                "INSERT INTO diet_food_aliases (source_id, alias_key, food_id) VALUES (?, ?, ?)",
            )
            .bind(&source_id)
            .bind(alias)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        }
        imported += 1;
    }

    tx.commit().await?;
    tracing::info!(
        source_id,
        imported,
        skipped = parsed.error_count,
        "committed food import"
    );

    Ok(ImportReport {
        source: sources::get_source(ctx, &source_id).await?,
        imported,
        skipped: parsed.error_count,
    })
}

// ---- Export (§7.1) ------------------------------------------------------------------------------

pub struct ExportRow {
    pub name: String,
    pub macros: Macros,
    pub aliases: Vec<String>,
}

pub fn write_csv(rows: &[ExportRow]) -> Vec<u8> {
    let mut w = csv::WriterBuilder::new().from_writer(Vec::new());
    w.write_record([
        "name",
        "kcal",
        "protein_g",
        "carbs_g",
        "fat_g",
        "fiber_g",
        "aliases",
    ])
    .expect("writing to an in-memory buffer never fails");
    for r in rows {
        w.write_record([
            r.name.clone(),
            r.macros.kcal.to_string(),
            r.macros.protein_g.to_string(),
            r.macros.carbs_g.to_string(),
            r.macros.fat_g.to_string(),
            r.macros.fiber_g.to_string(),
            r.aliases.join("|"),
        ])
        .expect("writing to an in-memory buffer never fails");
    }
    w.into_inner().expect("in-memory buffer flush never fails")
}

pub fn write_json(source_name: &str, rows: &[ExportRow]) -> Vec<u8> {
    let foods: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "name": r.name,
                "kcal": r.macros.kcal,
                "protein_g": r.macros.protein_g,
                "carbs_g": r.macros.carbs_g,
                "fat_g": r.macros.fat_g,
                "fiber_g": r.macros.fiber_g,
                "aliases": r.aliases,
            })
        })
        .collect();
    let envelope = json!({ "format": "lifeagent.foods.v1", "name": source_name, "foods": foods });
    serde_json::to_vec_pretty(&envelope).expect("JSON envelope always serializes")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn csv_bytes(s: &str) -> Vec<u8> {
        s.as_bytes().to_vec()
    }

    #[test]
    fn parses_a_simple_csv_file() {
        let bytes =
            csv_bytes("name,kcal,protein_g,carbs_g,fat_g,fiber_g\n熟米饭,116,2.6,25.9,0.3,0.3\n");
        let parsed = parse("test.csv", &bytes).unwrap();
        assert_eq!(parsed.format, ImportFormat::Csv);
        assert_eq!(parsed.valid_rows(), 1);
        assert_eq!(parsed.error_count, 0);
        assert_eq!(parsed.rows[0].name, "熟米饭");
        assert_eq!(parsed.rows[0].macros.kcal, 116.0);
    }

    #[test]
    fn strips_a_leading_bom() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"name,kcal,protein_g,carbs_g,fat_g\nx,100,1,1,1\n");
        let parsed = parse("test.csv", &bytes).unwrap();
        assert_eq!(parsed.valid_rows(), 1);
    }

    #[test]
    fn non_utf8_is_rejected_with_a_zh_message() {
        let bytes = vec![0xFF, 0xFE, 0x00];
        let err = parse("test.csv", &bytes).unwrap_err();
        assert!(matches!(err, FoodDataError::Invalid(m) if m.contains("UTF-8")));
    }

    #[test]
    fn header_synonyms_chinese_and_full_width_parens_all_map() {
        let bytes = csv_bytes("食物,能量(kcal),蛋白质,碳水,脂肪,纤维\n芹菜,16,0.8,3.3,0.2,1.4\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.valid_rows(), 1);
        assert_eq!(parsed.rows[0].macros.kcal, 16.0);
        assert_eq!(parsed.rows[0].macros.fiber_g, 1.4);
    }

    #[test]
    fn bare_energy_header_is_read_as_kcal() {
        let bytes = csv_bytes("name,能量,protein_g,carbs_g,fat_g\nx,100,1,1,1\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.rows[0].macros.kcal, 100.0);
    }

    #[test]
    fn kj_is_converted_to_kcal() {
        let bytes = csv_bytes("name,kj,protein_g,carbs_g,fat_g\nx,418.4,1,1,1\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert!((parsed.rows[0].macros.kcal - 100.0).abs() < 1e-6);
    }

    #[test]
    fn kcal_and_kj_disagreement_over_2_percent_warns_but_kcal_wins() {
        // kj/4.184 = 100, kcal given as 110: >2% off.
        let bytes = csv_bytes("name,kcal,kj,protein_g,carbs_g,fat_g\nx,110,418.4,1,1,1\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.rows[0].macros.kcal, 110.0);
        assert!(parsed.warnings.iter().any(|w| w.message.contains("kj")));
    }

    #[test]
    fn basis_g_scales_to_per_100g() {
        let bytes = csv_bytes("name,kcal,protein_g,carbs_g,fat_g,basis_g\nx,50,2,2,2,50\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.rows[0].macros.kcal, 100.0);
        assert_eq!(parsed.rows[0].macros.protein_g, 4.0);
    }

    #[test]
    fn basis_g_must_be_positive() {
        let bytes = csv_bytes(
            "name,kcal,protein_g,carbs_g,fat_g,basis_g\nok,100,0,25,0,100\nx,50,2,2,2,0\n",
        );
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.valid_rows(), 1);
        assert_eq!(parsed.error_count, 1);
    }

    #[test]
    fn trace_marker_becomes_zero_with_a_file_level_warning() {
        let bytes = csv_bytes("name,kcal,protein_g,carbs_g,fat_g\nx,100,Tr,1,1\ny,100,微量,1,1\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.valid_rows(), 2);
        assert_eq!(parsed.rows[0].macros.protein_g, 0.0);
        assert!(parsed
            .warnings
            .iter()
            .any(|w| w.message.contains("2 行使用了")));
    }

    #[test]
    fn not_measured_marker_in_required_column_is_an_error_in_fiber_is_default() {
        let bytes = csv_bytes(
            "name,kcal,protein_g,carbs_g,fat_g,fiber_g\nok,100,0,25,0,0\nx,100,—,1,1,—\n",
        );
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.valid_rows(), 1);
        assert_eq!(parsed.error_count, 1);

        let bytes = csv_bytes("name,kcal,protein_g,carbs_g,fat_g,fiber_g\nx,100,1,1,1,—\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.valid_rows(), 1);
        assert_eq!(parsed.rows[0].macros.fiber_g, 0.0);
    }

    #[test]
    fn other_non_numeric_text_is_always_an_error() {
        let bytes = csv_bytes("name,kcal,protein_g,carbs_g,fat_g\nok,100,0,25,0\nx,abc,1,1,1\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.valid_rows(), 1);
        assert_eq!(parsed.error_count, 1);
    }

    #[test]
    fn duplicate_name_key_in_file_is_a_row_error_keeping_the_first() {
        let bytes = csv_bytes("name,kcal,protein_g,carbs_g,fat_g\nx,100,1,1,1\nX,200,2,2,2\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.valid_rows(), 1);
        assert_eq!(parsed.rows[0].macros.kcal, 100.0);
        assert_eq!(parsed.error_count, 1);
    }

    #[test]
    fn alias_colliding_with_another_rows_name_is_dropped_with_a_warning() {
        let bytes = csv_bytes(
            "name,kcal,protein_g,carbs_g,fat_g,aliases\n米饭,100,1,1,1,白米饭\n白米饭,100,1,1,1,\n",
        );
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.valid_rows(), 2);
        assert!(parsed.rows[0].aliases.is_empty());
        assert!(parsed.warnings.iter().any(|w| w.message.contains("重复")));
    }

    #[test]
    fn alias_equal_to_own_name_is_dropped_silently() {
        // protein 0 + carbs 25 + fat 0 -> energy 100, matching kcal exactly (no energy warning).
        let bytes = csv_bytes("name,kcal,protein_g,carbs_g,fat_g,aliases\n米饭,100,0,25,0,米饭\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert!(parsed.rows[0].aliases.is_empty());
        assert_eq!(parsed.warning_count, 0);
    }

    #[test]
    fn energy_mismatch_is_a_warning_not_a_correction() {
        // e = 4*1 + 4*1 + 9*1 = 17; given 300 is far off but must NOT be corrected.
        let bytes = csv_bytes("name,kcal,protein_g,carbs_g,fat_g\nx,300,1,1,1\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.rows[0].macros.kcal, 300.0);
        assert!(parsed.warnings.iter().any(|w| w.message.contains("推算")));
    }

    #[test]
    fn full_width_digits_are_converted() {
        let bytes = csv_bytes("name,kcal,protein_g,carbs_g,fat_g\nx,１２．５,1,1,1\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.rows[0].macros.kcal, 12.5);
    }

    #[test]
    fn quoted_csv_fields_with_commas_and_newlines_parse() {
        let bytes = csv_bytes("name,kcal,protein_g,carbs_g,fat_g\n\"米饭, 蒸\",100,1,1,1\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.rows[0].name, "米饭, 蒸");
    }

    #[test]
    fn unknown_columns_produce_one_file_level_warning() {
        let bytes = csv_bytes("name,kcal,protein_g,carbs_g,fat_g,foo\nx,100,1,1,1,bar\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert!(parsed.warnings.iter().any(|w| w.message.contains("foo")));
    }

    #[test]
    fn json_bare_array_parses() {
        let json = r#"[{"name":"熟米饭","kcal":116,"protein_g":2.6,"carbs_g":25.9,"fat_g":0.3,"fiber_g":0.3,"aliases":["米饭"]}]"#;
        let parsed = parse("t.json", json.as_bytes()).unwrap();
        assert_eq!(parsed.format, ImportFormat::Json);
        assert_eq!(parsed.valid_rows(), 1);
        assert_eq!(parsed.rows[0].aliases, vec!["米饭"]);
        assert_eq!(parsed.suggested_name, "t");
    }

    #[test]
    fn json_envelope_uses_its_name_as_the_suggested_name() {
        let json = r#"{"format":"lifeagent.foods.v1","name":"中国食物成分表 第6版","foods":[
            {"name":"燕麦片","kj":1413,"protein_g":10.1,"carbs_g":77.4,"fat_g":0.2,"fiber_g":6.0}
        ]}"#;
        let parsed = parse("data.json", json.as_bytes()).unwrap();
        assert_eq!(parsed.suggested_name, "中国食物成分表 第6版");
        assert_eq!(parsed.valid_rows(), 1);
    }

    #[test]
    fn zero_valid_rows_fails_the_whole_import() {
        let bytes = csv_bytes("name,kcal,protein_g,carbs_g,fat_g\n,100,1,1,1\n");
        let err = parse("t.csv", &bytes).unwrap_err();
        assert!(matches!(err, FoodDataError::Invalid(_)));
    }

    #[test]
    fn issue_lists_are_capped_at_200_but_report_the_full_count() {
        let mut csv = "name,kcal,protein_g,carbs_g,fat_g\n".to_string();
        // One valid row so the import doesn't fail outright, plus 250 invalid ones.
        csv.push_str("ok,100,1,1,1\n");
        for i in 0..250 {
            csv.push_str(&format!(",100,1,1,1,{i}\n"));
        }
        let parsed = parse("t.csv", csv.as_bytes()).unwrap();
        assert_eq!(parsed.error_count, 250);
        assert_eq!(parsed.errors.len(), 200);
    }
}
