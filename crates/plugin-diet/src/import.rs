//! Batch import of a dataset source from CSV or JSON, and export back out, per
//! `docs/NUTRITION_SOURCES.md` §6/§7.1, extended by `docs/DISH_NUTRITION.md` §7 with per-serving
//! dish rows alongside per-100 g food rows.
//!
//! `parse` is pure (bytes in, rows and issues out); `preview` checks against the current DB
//! without writing; `commit` writes in one transaction. This split lets the CLI and the desktop
//! share one code path for preview and commit.

use std::collections::HashMap;
use std::collections::HashSet;

use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

use lifeagent_core::plugin::{PluginContext, PluginError};

use crate::foods::{normalize, validate_per_100g, MacroViolation};
use crate::model::Macros;
use crate::sources::{self, DatasetOrigin, SourceInfo};

/// File size limit (§6.2).
const MAX_BYTES: usize = 20 * 1024 * 1024;
/// Row count limit (§6.2).
const MAX_ROWS: usize = 50_000;
/// Issue lists are capped at this many entries in `ParsedImport`/`ImportPreview` (§6.2).
const MAX_ISSUES: usize = 200;
const MAX_ALIASES_PER_ROW: usize = 20;
/// `ImportPreview.brands` cap (`docs/DISH_NUTRITION.md` §7.3).
const MAX_BRANDS: usize = 20;

#[derive(Debug, thiserror::Error)]
pub enum FoodDataError {
    /// User-caused: bad file, limits, zero valid rows, rename length, …
    #[error("{0}")]
    Invalid(ImportMessage),
    /// Unknown source id, expired import token.
    #[error("{0}")]
    NotFound(ImportMessage),
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
    pub message: ImportMessage,
}

/// Everything an import can report to the user — file-level failures (`FoodDataError`) and
/// row-level errors and warnings (`RowIssue`) — as data, so the UI words it in the user's
/// language (`apps/web/src/i18n`). `Display` is the zh-CN text the CLI prints.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImportMessage {
    // ---- File level ----
    FileTooLarge {
        max_mb: usize,
    },
    NotUtf8,
    CsvHeader {
        error: String,
    },
    CsvRow {
        line: usize,
        error: String,
    },
    JsonParse {
        error: String,
    },
    JsonItemNotObject {
        line: usize,
    },
    JsonMissingRows,
    JsonFoodsNotArray,
    JsonDishesNotArray,
    JsonWrongShape,
    TooManyRows {
        max: usize,
    },
    NoValidRows,
    MissingFileContent,
    ImportExpired,
    SourceNameLength,
    NotADataset,
    SourceNotFound,
    // ---- Row level ----
    UnknownBasis {
        value: String,
    },
    /// `field` is the column name as written in the file format (`protein_g`, `kcal`, …).
    NotANumber {
        field: &'static str,
    },
    MissingField {
        field: &'static str,
    },
    MissingEnergy,
    NotPositive {
        field: &'static str,
        got: f64,
    },
    InvalidMacros {
        violation: MacroViolation,
    },
    KcalKjMismatch {
        kcal: f64,
        kcal_from_kj: f64,
    },
    EnergyMismatch {
        kcal: f64,
        estimated: f64,
    },
    TooManyAliases {
        max: usize,
    },
    BrandOnFoodRow,
    FoodNameEmpty,
    FoodNameTooLong {
        max: usize,
    },
    DishNameEmpty,
    DishNameTooLong {
        max: usize,
    },
    BrandTooLong {
        max: usize,
    },
    DuplicateFood,
    DuplicateDish,
    DuplicateFoodAlias {
        alias: String,
    },
    DuplicateDishAlias {
        alias: String,
    },
    TraceValues {
        rows: usize,
    },
    UnknownColumns {
        columns: Vec<String>,
    },
}

impl std::fmt::Display for ImportMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use ImportMessage::*;
        match self {
            FileTooLarge { max_mb } => write!(f, "文件超过 {max_mb} MB 限制"),
            NotUtf8 => write!(f, "文件不是 UTF-8 编码，请用 UTF-8 另存后重试"),
            CsvHeader { error } => write!(f, "CSV 表头解析失败：{error}"),
            CsvRow { line, error } => write!(f, "CSV 第 {line} 行解析失败：{error}"),
            JsonParse { error } => write!(f, "JSON 解析失败：{error}"),
            JsonItemNotObject { line } => write!(f, "第 {line} 项不是对象"),
            JsonMissingRows => write!(f, "JSON 对象缺少 foods 或 dishes 字段"),
            JsonFoodsNotArray => write!(f, "foods 必须是数组"),
            JsonDishesNotArray => write!(f, "dishes 必须是数组"),
            JsonWrongShape => write!(f, "JSON 必须是数组，或包含 foods/dishes 字段的对象"),
            TooManyRows { max } => write!(f, "文件行数超过 {max} 行限制"),
            NoValidRows => write!(f, "文件中没有可导入的有效数据行"),
            MissingFileContent => write!(f, "请求缺少文件内容。"),
            ImportExpired => write!(f, "导入已过期，请重新选择文件"),
            SourceNameLength => write!(f, "数据源名称需为 1-60 个字符"),
            NotADataset => write!(f, "只能替换数据集来源"),
            SourceNotFound => write!(f, "数据源不存在"),
            UnknownBasis { value } => {
                write!(f, "basis 列的值无法识别：{value:?}（应为 100g 或 serving）")
            }
            NotANumber { field } => write!(f, "{field} 不是数字"),
            MissingField { field } => write!(f, "缺少必填字段 {field}"),
            MissingEnergy => write!(f, "缺少必填字段 kcal 或 kj"),
            NotPositive { field, got } => write!(f, "{field} 必须大于 0，实际为 {got}"),
            InvalidMacros { violation } => write!(f, "{violation}"),
            KcalKjMismatch { kcal, kcal_from_kj } => write!(
                f,
                "kcal（{kcal:.0}）与 kj 换算得到的 {kcal_from_kj:.0} 相差超过 2%，以 kcal 为准"
            ),
            EnergyMismatch { kcal, estimated } => write!(
                f,
                "kcal（{kcal:.0}）与营养素推算的能量（{estimated:.0}）相差较大，导入的数值保持不变"
            ),
            TooManyAliases { max } => write!(f, "别名数超过 {max} 个，多余的已忽略"),
            BrandOnFoodRow => write!(
                f,
                "brand 列仅对菜品行（basis=serving）有效，本行（100g 基准）已忽略该列"
            ),
            FoodNameEmpty => write!(f, "食物名称不能为空"),
            FoodNameTooLong { max } => write!(f, "食物名称超过 {max} 字符"),
            DishNameEmpty => write!(f, "菜品名称不能为空"),
            DishNameTooLong { max } => write!(f, "菜品名称超过 {max} 字符"),
            BrandTooLong { max } => write!(f, "品牌名称超过 {max} 字符"),
            DuplicateFood => write!(f, "与文件中更早一行的食物名称重复"),
            DuplicateDish => write!(f, "与文件中更早一行的菜品（同品牌同名称）重复"),
            DuplicateFoodAlias { alias } => {
                write!(f, "别名“{alias}”与文件中其他食物名称或别名重复，已忽略")
            }
            DuplicateDishAlias { alias } => {
                write!(
                    f,
                    "别名“{alias}”与文件中同品牌的其他菜品名称或别名重复，已忽略"
                )
            }
            TraceValues { rows } => write!(f, "{rows} 行使用了“Tr/微量”标记，已按 0 处理"),
            UnknownColumns { columns } => write!(f, "忽略了未知列：{}", columns.join("、")),
        }
    }
}

#[derive(Debug, Clone)]
struct ValidFoodRow {
    line: usize,
    name: String,
    name_key: String,
    macros: Macros,
    aliases: Vec<String>,
}

#[derive(Debug, Clone)]
struct ValidDishRow {
    line: usize,
    brand: String,
    brand_key: String,
    name: String,
    name_key: String,
    serving_g: Option<f64>,
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
    food_rows: Vec<ValidFoodRow>,
    dish_rows: Vec<ValidDishRow>,
    pub errors: Vec<RowIssue>,
    pub error_count: usize,
    pub warnings: Vec<RowIssue>,
    pub warning_count: usize,
}

impl ParsedImport {
    pub fn valid_food_rows(&self) -> usize {
        self.food_rows.len()
    }

    pub fn valid_dish_rows(&self) -> usize {
        self.dish_rows.len()
    }

    /// Foods and dishes combined.
    pub fn valid_rows(&self) -> usize {
        self.food_rows.len() + self.dish_rows.len()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportRowView {
    /// `"100g"` or `"serving"`.
    pub basis: &'static str,
    /// Dish rows only.
    pub brand: Option<String>,
    /// Dish rows only.
    pub serving_g: Option<f64>,
    pub name: String,
    pub kcal: f64,
    pub protein_g: f64,
    pub carbs_g: f64,
    pub fat_g: f64,
    pub fiber_g: f64,
    pub aliases: Vec<String>,
}

impl From<&ValidFoodRow> for ImportRowView {
    fn from(row: &ValidFoodRow) -> Self {
        let r = row.macros.rounded();
        ImportRowView {
            basis: "100g",
            brand: None,
            serving_g: None,
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

impl From<&ValidDishRow> for ImportRowView {
    fn from(row: &ValidDishRow) -> Self {
        let r = row.macros.rounded();
        ImportRowView {
            basis: "serving",
            brand: (!row.brand.is_empty()).then(|| row.brand.clone()),
            serving_g: row.serving_g,
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
    pub valid_food_rows: usize,
    pub valid_dish_rows: usize,
    /// `valid_food_rows + valid_dish_rows`.
    pub valid_rows: usize,
    pub errors: Vec<RowIssue>,
    pub error_count: usize,
    pub warnings: Vec<RowIssue>,
    pub warning_count: usize,
    /// Valid food rows whose name the user has set by hand — they won't take effect while
    /// `manual` has them.
    pub shadowed_by_manual: usize,
    /// Valid food rows that will override an earlier AI estimate.
    pub replaces_estimates: usize,
    /// Valid dish rows whose `(brand, name)` the user has set by hand.
    pub dish_shadowed_by_manual: usize,
    /// Distinct dish brands in the file, non-empty, max 20 (§7.3).
    pub brands: Vec<String>,
    /// First 20 valid rows (foods and dishes, in file order), values rounded for display.
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
    pub imported_dishes: usize,
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

// ---- Row kind: food (per 100 g) or dish (per serving) (`docs/DISH_NUTRITION.md` §7.1) --------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowKind {
    Food,
    Dish,
}

/// The `basis`/`基准` column (or, for a bare JSON array, the per-row `"basis"` key): `100g` /
/// `每100g` (also `100 g`, whitespace-insensitive) selects a food row (the default when the
/// column is absent or blank); `serving` / `份` / `每份` selects a dish row.
fn parse_row_kind(raw: Option<&str>) -> Result<RowKind, ImportMessage> {
    let Some(s) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(RowKind::Food);
    };
    let norm: String = s
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase();
    match norm.as_str() {
        "100g" | "每100g" => Ok(RowKind::Food),
        "serving" | "份" | "每份" => Ok(RowKind::Dish),
        _ => Err(ImportMessage::UnknownBasis {
            value: s.to_string(),
        }),
    }
}

// ---- Column mapping (§6.1, extended by DISH_NUTRITION §7.1) -----------------------------------

#[derive(Debug, Clone, Copy)]
enum Column {
    Name,
    Kcal,
    Kj,
    Protein,
    Carbs,
    Fat,
    Fiber,
    /// `basis_g` / `每份克数`: a food row's scale-to-100g weight, or a dish row's `serving_g`
    /// (stored as-is, never used to scale).
    BasisG,
    /// `basis` / `基准`: which of the two the row is (§7.1).
    RowBasis,
    /// Dish rows only.
    Brand,
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
        ("basis_g", Column::BasisG),
        ("per_g", Column::BasisG),
        ("每份克数", Column::BasisG),
        ("basis", Column::RowBasis),
        ("基准", Column::RowBasis),
        ("brand", Column::Brand),
        ("品牌", Column::Brand),
        ("店铺", Column::Brand),
        ("ブランド", Column::Brand),
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
    brand: Option<String>,
    row_basis: Option<String>,
    kcal: CellValue,
    kj: CellValue,
    protein: CellValue,
    carbs: CellValue,
    fat: CellValue,
    fiber: CellValue,
    basis_g: CellValue,
    aliases: Vec<String>,
}

impl RawRow {
    fn empty(line: usize) -> Self {
        RawRow {
            line,
            name: None,
            brand: None,
            row_basis: None,
            kcal: CellValue::NotMeasured,
            kj: CellValue::NotMeasured,
            protein: CellValue::NotMeasured,
            carbs: CellValue::NotMeasured,
            fat: CellValue::NotMeasured,
            fiber: CellValue::NotMeasured,
            basis_g: CellValue::NotMeasured,
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
            Column::BasisG => self.basis_g = parse_cell(field),
            Column::RowBasis => self.row_basis = Some(field.to_string()),
            Column::Brand => self.brand = Some(field.to_string()),
            Column::Aliases => self.aliases = split_aliases(field),
        }
    }
}

/// A raw row paired with its resolved kind (or the "basis 列" text error that kept it from
/// resolving to one).
type KindedRow = (RawRow, Result<RowKind, ImportMessage>);

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
        .map_err(|e| {
            FoodDataError::Invalid(ImportMessage::CsvHeader {
                error: e.to_string(),
            })
        })?
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
        let record = record.map_err(|e| {
            FoodDataError::Invalid(ImportMessage::CsvRow {
                line: i + 2,
                error: e.to_string(),
            })
        })?;
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
    "serving_g",
    "basis",
    "brand",
    "aliases",
];

/// Which JSON array a row came from — this alone decides its `RowKind`, and which key
/// (`basis_g` vs `serving_g`) holds the gram value, per `docs/DISH_NUTRITION.md` §7.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RowArrayKind {
    /// A bare top-level JSON array: mixed rows, kind decided per-row by a `"basis"` key, like
    /// CSV.
    Mixed,
    /// The envelope's `"foods"` array: always a food row.
    Food,
    /// The envelope's `"dishes"` array: always a dish row.
    Dish,
}

fn parse_json_row_array(
    array: Vec<Value>,
    kind: RowArrayKind,
) -> Result<(Vec<RawRow>, Vec<String>), FoodDataError> {
    let mut unknown = HashSet::new();
    let mut rows = Vec::with_capacity(array.len());
    for (i, item) in array.into_iter().enumerate() {
        let line = i + 1;
        let Value::Object(obj) = item else {
            return Err(FoodDataError::Invalid(ImportMessage::JsonItemNotObject {
                line,
            }));
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
        row.basis_g = match kind {
            RowArrayKind::Dish => cell_from_json(obj.get("serving_g")),
            RowArrayKind::Mixed | RowArrayKind::Food => cell_from_json(obj.get("basis_g")),
        };
        if kind == RowArrayKind::Mixed {
            row.row_basis = obj
                .get("basis")
                .and_then(|v| v.as_str())
                .map(str::to_string);
        }
        row.brand = obj
            .get("brand")
            .and_then(|v| v.as_str())
            .map(str::to_string);
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
    Ok((rows, unknown.into_iter().collect()))
}

/// (envelope `name`, rows paired with their resolved kind, unknown top-level keys seen).
type ParsedJsonRows = (Option<String>, Vec<KindedRow>, Vec<String>);

fn parse_json(text: &str) -> Result<ParsedJsonRows, FoodDataError> {
    let value: Value = serde_json::from_str(text).map_err(|e| {
        FoodDataError::Invalid(ImportMessage::JsonParse {
            error: e.to_string(),
        })
    })?;
    match value {
        Value::Array(a) => {
            let (rows, unknown) = parse_json_row_array(a, RowArrayKind::Mixed)?;
            let kinded = rows
                .into_iter()
                .map(|r| {
                    let k = parse_row_kind(r.row_basis.as_deref());
                    (r, k)
                })
                .collect();
            Ok((None, kinded, unknown))
        }
        Value::Object(mut obj) => {
            let name = obj
                .remove("name")
                .and_then(|v| v.as_str().map(str::to_string));
            let foods_val = obj.remove("foods");
            let dishes_val = obj.remove("dishes");
            if foods_val.is_none() && dishes_val.is_none() {
                return Err(FoodDataError::Invalid(ImportMessage::JsonMissingRows));
            }

            let mut kinded = Vec::new();
            let mut unknown_all: HashSet<String> = HashSet::new();
            if let Some(v) = foods_val {
                let Value::Array(a) = v else {
                    return Err(FoodDataError::Invalid(ImportMessage::JsonFoodsNotArray));
                };
                let (rows, unknown) = parse_json_row_array(a, RowArrayKind::Food)?;
                unknown_all.extend(unknown);
                kinded.extend(rows.into_iter().map(|r| (r, Ok(RowKind::Food))));
            }
            if let Some(v) = dishes_val {
                let Value::Array(a) = v else {
                    return Err(FoodDataError::Invalid(ImportMessage::JsonDishesNotArray));
                };
                let (rows, unknown) = parse_json_row_array(a, RowArrayKind::Dish)?;
                unknown_all.extend(unknown);
                kinded.extend(rows.into_iter().map(|r| (r, Ok(RowKind::Dish))));
            }
            Ok((name, kinded, unknown_all.into_iter().collect()))
        }
        _ => Err(FoodDataError::Invalid(ImportMessage::JsonWrongShape)),
    }
}

// ---- Row-level validation (§6.2, extended by DISH_NUTRITION §7.2) -----------------------------

/// Resolves the required macros (protein/carbs/fat, fiber defaulting to 0) and kcal-or-kj,
/// exactly as stated in the row — no scaling. Shared by food rows (scaled by the caller
/// afterwards) and dish rows (used as-is: dish macros are already per serving).
fn resolve_macros(
    raw: &RawRow,
    issue: &impl Fn(ImportMessage) -> RowIssue,
    trace_rows: &mut usize,
) -> Result<(Macros, Vec<RowIssue>), RowIssue> {
    let mut warnings = Vec::new();

    let (protein, p_trace) = resolve_numeric(&raw.protein)
        .map_err(|_| issue(ImportMessage::NotANumber { field: "protein_g" }))?;
    let protein =
        protein.ok_or_else(|| issue(ImportMessage::MissingField { field: "protein_g" }))?;
    let (carbs, c_trace) = resolve_numeric(&raw.carbs)
        .map_err(|_| issue(ImportMessage::NotANumber { field: "carbs_g" }))?;
    let carbs = carbs.ok_or_else(|| issue(ImportMessage::MissingField { field: "carbs_g" }))?;
    let (fat, f_trace) = resolve_numeric(&raw.fat)
        .map_err(|_| issue(ImportMessage::NotANumber { field: "fat_g" }))?;
    let fat = fat.ok_or_else(|| issue(ImportMessage::MissingField { field: "fat_g" }))?;
    let (fiber, fi_trace) = resolve_numeric(&raw.fiber)
        .map_err(|_| issue(ImportMessage::NotANumber { field: "fiber_g" }))?;
    let fiber = fiber.unwrap_or(0.0);

    let (kcal_val, k_trace) = resolve_numeric(&raw.kcal)
        .map_err(|_| issue(ImportMessage::NotANumber { field: "kcal" }))?;
    let (kj_val, kj_trace) =
        resolve_numeric(&raw.kj).map_err(|_| issue(ImportMessage::NotANumber { field: "kj" }))?;
    let kcal_from_kj = kj_val.map(|kj| kj / 4.184);
    let kcal = match (kcal_val, kcal_from_kj) {
        (Some(k), Some(kj_k)) => {
            if (k - kj_k).abs() > 0.02 * k.abs().max(1.0) {
                warnings.push(issue(ImportMessage::KcalKjMismatch {
                    kcal: k,
                    kcal_from_kj: kj_k,
                }));
            }
            k
        }
        (Some(k), None) => k,
        (None, Some(kj_k)) => kj_k,
        (None, None) => return Err(issue(ImportMessage::MissingEnergy)),
    };
    for traced in [p_trace, c_trace, f_trace, fi_trace, k_trace, kj_trace] {
        if traced {
            *trace_rows += 1;
        }
    }

    Ok((
        Macros {
            kcal,
            protein_g: protein,
            carbs_g: carbs,
            fat_g: fat,
            fiber_g: fiber,
        },
        warnings,
    ))
}

fn energy_mismatch_warning(
    macros: &Macros,
    issue: &impl Fn(ImportMessage) -> RowIssue,
) -> Option<RowIssue> {
    let e = 4.0 * macros.protein_g
        + 4.0 * (macros.carbs_g - macros.fiber_g)
        + 9.0 * macros.fat_g
        + 2.0 * macros.fiber_g;
    if (macros.kcal - e).abs() > f64::max(20.0, 0.15 * e) {
        Some(issue(ImportMessage::EnergyMismatch {
            kcal: macros.kcal,
            estimated: e,
        }))
    } else {
        None
    }
}

fn resolve_row_aliases(
    raw_aliases: &[String],
    name_key: &str,
    warnings: &mut Vec<RowIssue>,
    issue: &impl Fn(ImportMessage) -> RowIssue,
) -> Vec<String> {
    let mut aliases = Vec::new();
    for alias_raw in raw_aliases.iter().take(MAX_ALIASES_PER_ROW) {
        let key = normalize(alias_raw);
        if key.is_empty() || key == name_key {
            continue;
        }
        aliases.push(key);
    }
    if raw_aliases.len() > MAX_ALIASES_PER_ROW {
        warnings.push(issue(ImportMessage::TooManyAliases {
            max: MAX_ALIASES_PER_ROW,
        }));
    }
    aliases
}

fn process_food_row(
    raw: RawRow,
    trace_rows: &mut usize,
) -> Result<(ValidFoodRow, Vec<RowIssue>), RowIssue> {
    let line = raw.line;
    let name = raw.name.clone().unwrap_or_default().trim().to_string();
    let issue = |message: ImportMessage| RowIssue {
        line,
        name: (!name.is_empty()).then(|| name.clone()),
        message,
    };
    let mut warnings = Vec::new();

    if raw.brand.as_deref().is_some_and(|b| !b.trim().is_empty()) {
        warnings.push(issue(ImportMessage::BrandOnFoodRow));
    }

    let (basis, basis_trace) = resolve_numeric(&raw.basis_g)
        .map_err(|_| issue(ImportMessage::NotANumber { field: "basis_g" }))?;
    let basis = basis.unwrap_or(100.0);
    if basis_trace {
        *trace_rows += 1;
    }
    if basis <= 0.0 {
        return Err(issue(ImportMessage::NotPositive {
            field: "basis_g",
            got: basis,
        }));
    }

    let (raw_macros, macro_warnings) = resolve_macros(&raw, &issue, trace_rows)?;
    warnings.extend(macro_warnings);

    let scale = 100.0 / basis;
    let macros = Macros {
        kcal: raw_macros.kcal * scale,
        protein_g: raw_macros.protein_g * scale,
        carbs_g: raw_macros.carbs_g * scale,
        fat_g: raw_macros.fat_g * scale,
        fiber_g: raw_macros.fiber_g * scale,
    };
    validate_per_100g(&macros)
        .map_err(|violation| issue(ImportMessage::InvalidMacros { violation }))?;

    let name_key = normalize(&name);
    if name_key.is_empty() {
        return Err(issue(ImportMessage::FoodNameEmpty));
    }
    if name.chars().count() > 100 {
        return Err(issue(ImportMessage::FoodNameTooLong { max: 100 }));
    }

    warnings.extend(energy_mismatch_warning(&macros, &issue));
    let aliases = resolve_row_aliases(&raw.aliases, &name_key, &mut warnings, &issue);

    Ok((
        ValidFoodRow {
            line,
            name,
            name_key,
            macros,
            aliases,
        },
        warnings,
    ))
}

fn process_dish_row(
    raw: RawRow,
    trace_rows: &mut usize,
) -> Result<(ValidDishRow, Vec<RowIssue>), RowIssue> {
    let line = raw.line;
    let name = raw.name.clone().unwrap_or_default().trim().to_string();
    let issue = |message: ImportMessage| RowIssue {
        line,
        name: (!name.is_empty()).then(|| name.clone()),
        message,
    };
    let mut warnings = Vec::new();

    let (serving_g, basis_trace) = resolve_numeric(&raw.basis_g)
        .map_err(|_| issue(ImportMessage::NotANumber { field: "serving_g" }))?;
    if basis_trace {
        *trace_rows += 1;
    }
    if let Some(g) = serving_g {
        if g <= 0.0 {
            return Err(issue(ImportMessage::NotPositive {
                field: "serving_g",
                got: g,
            }));
        }
    }

    let (macros, macro_warnings) = resolve_macros(&raw, &issue, trace_rows)?;
    warnings.extend(macro_warnings);

    sources::validate_dish_macros(&macros, serving_g)
        .map_err(|violation| issue(ImportMessage::InvalidMacros { violation }))?;

    let name_key = normalize(&name);
    if name_key.is_empty() {
        return Err(issue(ImportMessage::DishNameEmpty));
    }
    if name.chars().count() > 100 {
        return Err(issue(ImportMessage::DishNameTooLong { max: 100 }));
    }

    let brand = raw.brand.clone().unwrap_or_default().trim().to_string();
    if brand.chars().count() > 60 {
        return Err(issue(ImportMessage::BrandTooLong { max: 60 }));
    }
    let brand_key = normalize(&brand);

    warnings.extend(energy_mismatch_warning(&macros, &issue));
    let aliases = resolve_row_aliases(&raw.aliases, &name_key, &mut warnings, &issue);

    Ok((
        ValidDishRow {
            line,
            brand,
            brand_key,
            name,
            name_key,
            serving_g,
            macros,
            aliases,
        },
        warnings,
    ))
}

/// Pure: bytes in, rows and issues out. No DB, no clock, no model.
pub fn parse(file_name: &str, bytes: &[u8]) -> Result<ParsedImport, FoodDataError> {
    if bytes.len() > MAX_BYTES {
        return Err(FoodDataError::Invalid(ImportMessage::FileTooLarge {
            max_mb: MAX_BYTES / (1024 * 1024),
        }));
    }
    let text =
        std::str::from_utf8(bytes).map_err(|_| FoodDataError::Invalid(ImportMessage::NotUtf8))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    let format = detect_format(file_name, text);
    let (envelope_name, kinded_rows, unknown_columns): (
        Option<String>,
        Vec<KindedRow>,
        Vec<String>,
    ) = match format {
        ImportFormat::Csv => {
            let (rows, unknown) = parse_csv(text)?;
            let kinded = rows
                .into_iter()
                .map(|r| {
                    let k = parse_row_kind(r.row_basis.as_deref());
                    (r, k)
                })
                .collect();
            (None, kinded, unknown)
        }
        ImportFormat::Json => parse_json(text)?,
    };
    if kinded_rows.len() > MAX_ROWS {
        return Err(FoodDataError::Invalid(ImportMessage::TooManyRows {
            max: MAX_ROWS,
        }));
    }
    let suggested_name = envelope_name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| file_stem(file_name));

    let total_rows = kinded_rows.len();
    let mut trace_rows = 0usize;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut food_candidates = Vec::new();
    let mut dish_candidates = Vec::new();
    let mut seen_food_keys = HashSet::new();
    let mut seen_dish_keys = HashSet::new();

    for (raw, kind_result) in kinded_rows {
        let line = raw.line;
        let raw_name = raw.name.clone();
        let kind = match kind_result {
            Ok(k) => k,
            Err(message) => {
                errors.push(RowIssue {
                    line,
                    name: raw_name,
                    message,
                });
                continue;
            }
        };
        match kind {
            RowKind::Food => match process_food_row(raw, &mut trace_rows) {
                Ok((row, row_warnings)) => {
                    if !seen_food_keys.insert(row.name_key.clone()) {
                        errors.push(RowIssue {
                            line: row.line,
                            name: Some(row.name.clone()),
                            message: ImportMessage::DuplicateFood,
                        });
                        continue;
                    }
                    warnings.extend(row_warnings);
                    food_candidates.push(row);
                }
                Err(e) => errors.push(e),
            },
            RowKind::Dish => match process_dish_row(raw, &mut trace_rows) {
                Ok((row, row_warnings)) => {
                    if !seen_dish_keys.insert((row.brand_key.clone(), row.name_key.clone())) {
                        errors.push(RowIssue {
                            line: row.line,
                            name: Some(row.name.clone()),
                            message: ImportMessage::DuplicateDish,
                        });
                        continue;
                    }
                    warnings.extend(row_warnings);
                    dish_candidates.push(row);
                }
                Err(e) => errors.push(e),
            },
        }
    }
    if trace_rows > 0 {
        warnings.push(RowIssue {
            line: 0,
            name: None,
            message: ImportMessage::TraceValues { rows: trace_rows },
        });
    }
    if !unknown_columns.is_empty() {
        let mut cols = unknown_columns;
        cols.sort();
        warnings.push(RowIssue {
            line: 0,
            name: None,
            message: ImportMessage::UnknownColumns { columns: cols },
        });
    }

    // Cross-row alias de-duplication (§6.2): an alias equal to another row's name or alias in
    // the same file is dropped. `claimed` starts with every kept row's own name, so an alias can
    // never shadow a real name in the same file. Food and dish aliases live in separate
    // namespaces; a dish's is additionally scoped to its own brand (a "牛丼" alias under 松屋
    // never collides with a same-named dish under 吉野家).
    let mut claimed: HashSet<String> = food_candidates.iter().map(|r| r.name_key.clone()).collect();
    let mut food_rows = Vec::with_capacity(food_candidates.len());
    for mut row in food_candidates {
        let mut kept = Vec::new();
        for alias in row.aliases.drain(..) {
            if claimed.contains(&alias) {
                warnings.push(RowIssue {
                    line: row.line,
                    name: Some(row.name.clone()),
                    message: ImportMessage::DuplicateFoodAlias {
                        alias: alias.clone(),
                    },
                });
                continue;
            }
            claimed.insert(alias.clone());
            kept.push(alias);
        }
        row.aliases = kept;
        food_rows.push(row);
    }

    let mut claimed_by_brand: HashMap<String, HashSet<String>> = HashMap::new();
    for row in &dish_candidates {
        claimed_by_brand
            .entry(row.brand_key.clone())
            .or_default()
            .insert(row.name_key.clone());
    }
    let mut dish_rows = Vec::with_capacity(dish_candidates.len());
    for mut row in dish_candidates {
        let claimed = claimed_by_brand.entry(row.brand_key.clone()).or_default();
        let mut kept = Vec::new();
        for alias in row.aliases.drain(..) {
            if claimed.contains(&alias) {
                warnings.push(RowIssue {
                    line: row.line,
                    name: Some(row.name.clone()),
                    message: ImportMessage::DuplicateDishAlias {
                        alias: alias.clone(),
                    },
                });
                continue;
            }
            claimed.insert(alias.clone());
            kept.push(alias);
        }
        row.aliases = kept;
        dish_rows.push(row);
    }

    if food_rows.is_empty() && dish_rows.is_empty() {
        return Err(FoodDataError::Invalid(ImportMessage::NoValidRows));
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
        food_rows,
        dish_rows,
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
    for row in &parsed.food_rows {
        let manual: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM diet_foods WHERE source_id = 'manual' AND name_key = $1",
        )
        .bind(&row.name_key)
        .fetch_optional(&ctx.db)
        .await?;
        if manual.is_some() {
            shadowed_by_manual += 1;
        }
        let estimate: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM diet_foods WHERE source_id = 'llm_estimate' AND name_key = $1",
        )
        .bind(&row.name_key)
        .fetch_optional(&ctx.db)
        .await?;
        if estimate.is_some() {
            replaces_estimates += 1;
        }
    }

    let mut dish_shadowed_by_manual = 0usize;
    for row in &parsed.dish_rows {
        let manual: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM diet_dishes WHERE source_id = 'manual' AND brand_key = $1 AND name_key = $2",
        )
        .bind(&row.brand_key)
        .bind(&row.name_key)
        .fetch_optional(&ctx.db)
        .await?;
        if manual.is_some() {
            dish_shadowed_by_manual += 1;
        }
    }

    let mut brands: Vec<String> = parsed
        .dish_rows
        .iter()
        .map(|r| r.brand.clone())
        .filter(|b| !b.is_empty())
        .collect();
    brands.sort();
    brands.dedup();
    brands.truncate(MAX_BRANDS);

    let mut sample_rows: Vec<(usize, ImportRowView)> = parsed
        .food_rows
        .iter()
        .map(|r| (r.line, ImportRowView::from(r)))
        .chain(
            parsed
                .dish_rows
                .iter()
                .map(|r| (r.line, ImportRowView::from(r))),
        )
        .collect();
    sample_rows.sort_by_key(|(line, _)| *line);

    Ok(ImportPreview {
        format: parsed.format,
        suggested_name: parsed.suggested_name.clone(),
        total_rows: parsed.total_rows,
        valid_food_rows: parsed.food_rows.len(),
        valid_dish_rows: parsed.dish_rows.len(),
        valid_rows: parsed.valid_rows(),
        errors: parsed.errors.clone(),
        error_count: parsed.error_count,
        warnings: parsed.warnings.clone(),
        warning_count: parsed.warning_count,
        shadowed_by_manual,
        replaces_estimates,
        dish_shadowed_by_manual,
        brands,
        sample: sample_rows.into_iter().take(20).map(|(_, v)| v).collect(),
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
        rows: (parsed.food_rows.len() + parsed.dish_rows.len()) as i64,
    })
    .expect("DatasetOrigin always serializes");

    let mut tx = ctx.db.begin().await?;
    let mut carried_food_counts: HashMap<String, i64> = HashMap::new();
    let mut carried_dish_counts: HashMap<(String, String), i64> = HashMap::new();

    let source_id = match &target {
        ImportTarget::New { name } => {
            let name = name.trim();
            if name.is_empty() || name.chars().count() > 60 {
                return Err(FoodDataError::Invalid(ImportMessage::SourceNameLength));
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
                 VALUES ($1, 'dataset', $2, $3, 1, $4, $5, $6)",
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
                sqlx::query_as("SELECT kind FROM diet_food_sources WHERE id = $1")
                    .bind(source_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            match kind {
                Some((k,)) if k == "dataset" => {}
                Some(_) => return Err(FoodDataError::Invalid(ImportMessage::NotADataset)),
                None => return Err(FoodDataError::NotFound(ImportMessage::SourceNotFound)),
            }
            let old_foods: Vec<(String, i64)> =
                sqlx::query_as("SELECT name_key, use_count FROM diet_foods WHERE source_id = $1")
                    .bind(source_id)
                    .fetch_all(&mut *tx)
                    .await?;
            carried_food_counts = old_foods.into_iter().collect();
            let old_dishes: Vec<(String, String, i64)> = sqlx::query_as(
                "SELECT brand_key, name_key, use_count FROM diet_dishes WHERE source_id = $1",
            )
            .bind(source_id)
            .fetch_all(&mut *tx)
            .await?;
            carried_dish_counts = old_dishes
                .into_iter()
                .map(|(b, n, u)| ((b, n), u))
                .collect();
            sqlx::query("DELETE FROM diet_foods WHERE source_id = $1")
                .bind(source_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM diet_dishes WHERE source_id = $1")
                .bind(source_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "UPDATE diet_food_sources SET origin_json = $1, updated_at = $2 WHERE id = $3",
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
    for row in &parsed.food_rows {
        let id = Uuid::now_v7().to_string();
        let use_count = carried_food_counts.get(&row.name_key).copied().unwrap_or(0);
        sqlx::query(
            "INSERT INTO diet_foods
                (id, source_id, name, name_key, kcal, protein_g, carbs_g, fat_g, fiber_g,
                 use_count, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
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
                "INSERT INTO diet_food_aliases (source_id, alias_key, food_id) VALUES ($1, $2, $3)",
            )
            .bind(&source_id)
            .bind(alias)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        }
        imported += 1;
    }

    let mut imported_dishes = 0usize;
    for row in &parsed.dish_rows {
        let id = Uuid::now_v7().to_string();
        let use_count = carried_dish_counts
            .get(&(row.brand_key.clone(), row.name_key.clone()))
            .copied()
            .unwrap_or(0);
        sqlx::query(
            "INSERT INTO diet_dishes
                (id, source_id, brand, brand_key, name, name_key, serving_g, kcal, protein_g,
                 carbs_g, fat_g, fiber_g, use_count, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
        )
        .bind(&id)
        .bind(&source_id)
        .bind(&row.brand)
        .bind(&row.brand_key)
        .bind(&row.name)
        .bind(&row.name_key)
        .bind(row.serving_g)
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
                "INSERT INTO diet_dish_source_aliases (source_id, brand_key, alias_key, dish_id)
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(&source_id)
            .bind(&row.brand_key)
            .bind(alias)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        }
        imported_dishes += 1;
    }

    tx.commit().await?;
    tracing::info!(
        source_id,
        imported,
        imported_dishes,
        skipped = parsed.error_count,
        "committed food/dish import"
    );

    Ok(ImportReport {
        source: sources::get_source(ctx, &source_id).await?,
        imported,
        imported_dishes,
        skipped: parsed.error_count,
    })
}

// ---- Export (§7.1) ------------------------------------------------------------------------------

pub struct ExportRow {
    /// `"100g"` or `"serving"`.
    pub basis: &'static str,
    /// `""` for a food row or a brandless dish.
    pub brand: String,
    /// Dish rows only.
    pub serving_g: Option<f64>,
    pub name: String,
    pub macros: Macros,
    pub aliases: Vec<String>,
}

/// One CSV, mixing food and dish rows (`basis` tells them apart, exactly as import reads it
/// back). `basis_g` is dish rows' `serving_g`, kept under its import-compatible column name.
pub fn write_csv(rows: &[ExportRow]) -> Vec<u8> {
    let mut w = csv::WriterBuilder::new().from_writer(Vec::new());
    w.write_record([
        "basis",
        "brand",
        "name",
        "kcal",
        "protein_g",
        "carbs_g",
        "fat_g",
        "fiber_g",
        "basis_g",
        "aliases",
    ])
    .expect("writing to an in-memory buffer never fails");
    for r in rows {
        w.write_record([
            r.basis.to_string(),
            r.brand.clone(),
            r.name.clone(),
            r.macros.kcal.to_string(),
            r.macros.protein_g.to_string(),
            r.macros.carbs_g.to_string(),
            r.macros.fat_g.to_string(),
            r.macros.fiber_g.to_string(),
            r.serving_g.map(|g| g.to_string()).unwrap_or_default(),
            r.aliases.join("|"),
        ])
        .expect("writing to an in-memory buffer never fails");
    }
    w.into_inner().expect("in-memory buffer flush never fails")
}

/// `foods`/`dishes` arrays, matching the envelope shape `parse_json` reads (§7.1).
pub fn write_json(source_name: &str, food_rows: &[ExportRow], dish_rows: &[ExportRow]) -> Vec<u8> {
    let foods: Vec<Value> = food_rows
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
    let mut envelope =
        json!({ "format": "lifeagent.foods.v1", "name": source_name, "foods": foods });
    if !dish_rows.is_empty() {
        let dishes: Vec<Value> = dish_rows
            .iter()
            .map(|r| {
                json!({
                    "brand": if r.brand.is_empty() { Value::Null } else { json!(r.brand) },
                    "name": r.name,
                    "serving_g": r.serving_g,
                    "kcal": r.macros.kcal,
                    "protein_g": r.macros.protein_g,
                    "carbs_g": r.macros.carbs_g,
                    "fat_g": r.macros.fat_g,
                    "fiber_g": r.macros.fiber_g,
                    "aliases": r.aliases,
                })
            })
            .collect();
        envelope["dishes"] = json!(dishes);
    }
    serde_json::to_vec_pretty(&envelope).expect("JSON envelope always serializes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::env;

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
        assert_eq!(parsed.food_rows[0].name, "熟米饭");
        assert_eq!(parsed.food_rows[0].macros.kcal, 116.0);
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
        assert!(matches!(err, FoodDataError::Invalid(m) if m.to_string().contains("UTF-8")));
    }

    #[test]
    fn header_synonyms_chinese_and_full_width_parens_all_map() {
        let bytes = csv_bytes("食物,能量(kcal),蛋白质,碳水,脂肪,纤维\n芹菜,16,0.8,3.3,0.2,1.4\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.valid_rows(), 1);
        assert_eq!(parsed.food_rows[0].macros.kcal, 16.0);
        assert_eq!(parsed.food_rows[0].macros.fiber_g, 1.4);
    }

    #[test]
    fn bare_energy_header_is_read_as_kcal() {
        let bytes = csv_bytes("name,能量,protein_g,carbs_g,fat_g\nx,100,1,1,1\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.food_rows[0].macros.kcal, 100.0);
    }

    #[test]
    fn kj_is_converted_to_kcal() {
        let bytes = csv_bytes("name,kj,protein_g,carbs_g,fat_g\nx,418.4,1,1,1\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert!((parsed.food_rows[0].macros.kcal - 100.0).abs() < 1e-6);
    }

    #[test]
    fn kcal_and_kj_disagreement_over_2_percent_warns_but_kcal_wins() {
        // kj/4.184 = 100, kcal given as 110: >2% off.
        let bytes = csv_bytes("name,kcal,kj,protein_g,carbs_g,fat_g\nx,110,418.4,1,1,1\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.food_rows[0].macros.kcal, 110.0);
        assert!(parsed
            .warnings
            .iter()
            .any(|w| w.message.to_string().contains("kj")));
    }

    #[test]
    fn basis_g_scales_to_per100g() {
        let bytes = csv_bytes("name,kcal,protein_g,carbs_g,fat_g,basis_g\nx,50,2,2,2,50\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.food_rows[0].macros.kcal, 100.0);
        assert_eq!(parsed.food_rows[0].macros.protein_g, 4.0);
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
        assert_eq!(parsed.food_rows[0].macros.protein_g, 0.0);
        assert!(parsed
            .warnings
            .iter()
            .any(|w| w.message.to_string().contains("2 行使用了")));
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
        assert_eq!(parsed.food_rows[0].macros.fiber_g, 0.0);
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
        assert_eq!(parsed.food_rows[0].macros.kcal, 100.0);
        assert_eq!(parsed.error_count, 1);
    }

    #[test]
    fn alias_colliding_with_another_rows_name_is_dropped_with_a_warning() {
        let bytes = csv_bytes(
            "name,kcal,protein_g,carbs_g,fat_g,aliases\n米饭,100,1,1,1,白米饭\n白米饭,100,1,1,1,\n",
        );
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.valid_rows(), 2);
        assert!(parsed.food_rows[0].aliases.is_empty());
        assert!(parsed
            .warnings
            .iter()
            .any(|w| w.message.to_string().contains("重复")));
    }

    #[test]
    fn alias_equal_to_own_name_is_dropped_silently() {
        // protein 0 + carbs 25 + fat 0 -> energy 100, matching kcal exactly (no energy warning).
        let bytes = csv_bytes("name,kcal,protein_g,carbs_g,fat_g,aliases\n米饭,100,0,25,0,米饭\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert!(parsed.food_rows[0].aliases.is_empty());
        assert_eq!(parsed.warning_count, 0);
    }

    #[test]
    fn energy_mismatch_is_a_warning_not_a_correction() {
        // e = 4*1 + 4*1 + 9*1 = 17; given 300 is far off but must NOT be corrected.
        let bytes = csv_bytes("name,kcal,protein_g,carbs_g,fat_g\nx,300,1,1,1\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.food_rows[0].macros.kcal, 300.0);
        assert!(parsed
            .warnings
            .iter()
            .any(|w| w.message.to_string().contains("推算")));
    }

    #[test]
    fn full_width_digits_are_converted() {
        let bytes = csv_bytes("name,kcal,protein_g,carbs_g,fat_g\nx,１２．５,1,1,1\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.food_rows[0].macros.kcal, 12.5);
    }

    #[test]
    fn quoted_csv_fields_with_commas_and_newlines_parse() {
        let bytes = csv_bytes("name,kcal,protein_g,carbs_g,fat_g\n\"米饭, 蒸\",100,1,1,1\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.food_rows[0].name, "米饭, 蒸");
    }

    #[test]
    fn unknown_columns_produce_one_file_level_warning() {
        let bytes = csv_bytes("name,kcal,protein_g,carbs_g,fat_g,foo\nx,100,1,1,1,bar\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert!(parsed
            .warnings
            .iter()
            .any(|w| w.message.to_string().contains("foo")));
    }

    #[test]
    fn json_bare_array_parses() {
        let json = r#"[{"name":"熟米饭","kcal":116,"protein_g":2.6,"carbs_g":25.9,"fat_g":0.3,"fiber_g":0.3,"aliases":["米饭"]}]"#;
        let parsed = parse("t.json", json.as_bytes()).unwrap();
        assert_eq!(parsed.format, ImportFormat::Json);
        assert_eq!(parsed.valid_rows(), 1);
        assert_eq!(parsed.food_rows[0].aliases, vec!["米饭"]);
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

    // ---- Dish rows (`docs/DISH_NUTRITION.md` §7.1/§7.2) ----------------------------------------

    #[test]
    fn basis_column_synonyms_select_food_or_dish_row_kind() {
        let bytes = csv_bytes(
            "name,brand,basis,kcal,protein_g,carbs_g,fat_g\n\
             米饭,,100g,100,1,1,1\n\
             粥,,每100g,50,1,1,1\n\
             牛めし,松屋,serving,733,25,90,28\n\
             味噌汁,松屋,份,40,3,5,1\n\
             定食,,每份,600,30,70,20\n",
        );
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.error_count, 0);
        assert_eq!(parsed.valid_food_rows(), 2);
        assert_eq!(parsed.valid_dish_rows(), 3);
    }

    #[test]
    fn unrecognized_basis_value_is_a_row_error() {
        let bytes = csv_bytes(
            "name,basis,kcal,protein_g,carbs_g,fat_g\nok,100g,100,1,1,1\nx,每斤,100,1,1,1\n",
        );
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.valid_food_rows(), 1);
        assert_eq!(parsed.error_count, 1);
        assert!(parsed.errors[0].message.to_string().contains("basis"));
    }

    #[test]
    fn brand_on_a_100g_row_is_dropped_with_a_warning() {
        let bytes = csv_bytes("name,brand,kcal,protein_g,carbs_g,fat_g\n米饭,金龙鱼,100,1,1,1\n");
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.valid_food_rows(), 1);
        assert!(parsed
            .warnings
            .iter()
            .any(|w| w.message.to_string().contains("brand")));
    }

    #[test]
    fn serving_g_is_stored_but_never_used_to_scale_dish_macros() {
        let bytes = csv_bytes(
            "name,brand,basis,basis_g,kcal,protein_g,carbs_g,fat_g\n牛めし（並盛）,松屋,serving,350,733,25,90,28\n",
        );
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.dish_rows[0].serving_g, Some(350.0));
        assert_eq!(parsed.dish_rows[0].macros.kcal, 733.0);
        assert_eq!(parsed.dish_rows[0].macros.protein_g, 25.0);
    }

    #[test]
    fn dish_row_validation_hard_rules() {
        let header = "name,brand,basis,basis_g,kcal,protein_g,carbs_g,fat_g,fiber_g\n";
        let ok = "ok,松屋,serving,350,700,25,90,28,3\n";

        // Missing required field (protein_g).
        let bytes = csv_bytes(&format!("{header}{ok}x,松屋,serving,,700,,90,28,3\n"));
        assert_eq!(parse("t.csv", &bytes).unwrap().error_count, 1);

        // fiber_g > carbs_g.
        let bytes = csv_bytes(&format!("{header}{ok}x,松屋,serving,,700,25,5,28,10\n"));
        assert_eq!(parse("t.csv", &bytes).unwrap().error_count, 1);

        // kcal > 5000 per serving.
        let bytes = csv_bytes(&format!("{header}{ok}x,松屋,serving,,5001,25,90,28,3\n"));
        assert_eq!(parse("t.csv", &bytes).unwrap().error_count, 1);

        // serving_g <= 0.
        let bytes = csv_bytes(&format!("{header}{ok}x,松屋,serving,0,700,25,90,28,3\n"));
        assert_eq!(parse("t.csv", &bytes).unwrap().error_count, 1);

        // protein + carbs + fat > serving_g.
        let bytes = csv_bytes(&format!("{header}{ok}x,松屋,serving,100,700,25,90,28,3\n"));
        assert_eq!(parse("t.csv", &bytes).unwrap().error_count, 1);

        // kcal > 9 x serving_g.
        let bytes = csv_bytes(&format!("{header}{ok}x,松屋,serving,50,700,1,1,1,0\n"));
        assert_eq!(parse("t.csv", &bytes).unwrap().error_count, 1);
    }

    #[test]
    fn same_dish_name_under_two_brands_is_fine_but_duplicate_within_one_brand_is_an_error() {
        let bytes = csv_bytes(
            "name,brand,basis,basis_g,kcal,protein_g,carbs_g,fat_g\n\
             牛丼,松屋,serving,,700,25,90,28\n\
             牛丼,吉野家,serving,,650,24,85,26\n\
             牛丼,松屋,serving,,720,26,92,29\n",
        );
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.valid_dish_rows(), 2);
        assert_eq!(parsed.error_count, 1);
        assert!(parsed.errors[0].message.to_string().contains("重复"));
    }

    #[test]
    fn dish_alias_collisions_are_scoped_per_brand() {
        let bytes = csv_bytes(
            "name,brand,basis,kcal,protein_g,carbs_g,fat_g,aliases\n\
             牛めし（並盛）,松屋,serving,733,25,90,28,牛丼\n\
             牛丼,吉野家,serving,650,24,85,26,\n",
        );
        let parsed = parse("t.csv", &bytes).unwrap();
        // "牛丼" is an alias under 松屋 and a real dish name under 吉野家 — different brand
        // scopes, so no collision.
        assert_eq!(parsed.dish_rows[0].aliases, vec![normalize("牛丼")]);
        assert_eq!(parsed.warning_count, 0);
    }

    #[test]
    fn dish_alias_colliding_within_the_same_brand_is_dropped_with_a_warning() {
        let bytes = csv_bytes(
            "name,brand,basis,kcal,protein_g,carbs_g,fat_g,aliases\n\
             牛めし（並盛）,松屋,serving,733,25,90,28,牛丼\n\
             牛丼,松屋,serving,650,24,85,26,\n",
        );
        let parsed = parse("t.csv", &bytes).unwrap();
        assert_eq!(parsed.valid_dish_rows(), 2);
        let matched = parsed
            .dish_rows
            .iter()
            .find(|r| r.name == "牛めし（並盛）")
            .unwrap();
        assert!(matched.aliases.is_empty());
        assert!(parsed
            .warnings
            .iter()
            .any(|w| w.message.to_string().contains("重复")));
    }

    #[test]
    fn json_envelope_with_a_dishes_array_parses() {
        let json = r#"{"format":"lifeagent.foods.v1","name":"松屋菜单","dishes":[
            {"brand":"松屋","name":"牛めし（並盛）","serving_g":350,"kcal":733,"protein_g":25,
             "carbs_g":90,"fat_g":28,"aliases":["牛丼並盛"]}
        ]}"#;
        let parsed = parse("t.json", json.as_bytes()).unwrap();
        assert_eq!(parsed.valid_dish_rows(), 1);
        assert_eq!(parsed.dish_rows[0].brand, "松屋");
        assert_eq!(parsed.dish_rows[0].serving_g, Some(350.0));
        assert_eq!(parsed.dish_rows[0].aliases, vec![normalize("牛丼並盛")]);
    }

    #[test]
    fn json_bare_array_with_basis_field_mixes_food_and_dish_rows() {
        let json = r#"[
            {"name":"熟米饭","kcal":116,"protein_g":2.6,"carbs_g":25.9,"fat_g":0.3},
            {"name":"牛めし（並盛）","brand":"松屋","basis":"serving","basis_g":350,
             "kcal":733,"protein_g":25,"carbs_g":90,"fat_g":28}
        ]"#;
        let parsed = parse("t.json", json.as_bytes()).unwrap();
        assert_eq!(parsed.valid_food_rows(), 1);
        assert_eq!(parsed.valid_dish_rows(), 1);
        assert_eq!(parsed.dish_rows[0].macros.kcal, 733.0);
        assert_eq!(parsed.dish_rows[0].serving_g, Some(350.0));
    }

    // ---- DB-backed: preview, commit, replace, export round trip --------------------------------

    #[tokio::test]
    async fn preview_reports_dish_counts_brands_and_shadowing() {
        let env = env().await;
        sources::upsert_manual_dish(
            &env.ctx,
            "牛めし（並盛）",
            Some("松屋"),
            crate::testutil::EGG,
            None,
        )
        .await
        .unwrap();
        let bytes = csv_bytes(
            "name,brand,basis,kcal,protein_g,carbs_g,fat_g\n\
             熟米饭,,100g,116,2.6,25.9,0.3\n\
             牛めし（並盛）,松屋,serving,733,25,90,28\n\
             味噌汁,松屋,serving,40,3,5,1\n",
        );
        let parsed = parse("t.csv", &bytes).unwrap();

        let preview = preview(&env.ctx, &parsed).await.unwrap();

        assert_eq!(preview.valid_food_rows, 1);
        assert_eq!(preview.valid_dish_rows, 2);
        assert_eq!(preview.valid_rows, 3);
        assert_eq!(preview.brands, vec!["松屋".to_string()]);
        assert_eq!(preview.dish_shadowed_by_manual, 1);
        assert_eq!(preview.sample.len(), 3);
        assert_eq!(preview.sample[0].basis, "100g");
        assert_eq!(preview.sample[1].basis, "serving");
    }

    #[tokio::test]
    async fn commit_new_dataset_inserts_dishes_and_their_source_aliases() {
        let env = env().await;
        let bytes = csv_bytes(
            "name,brand,basis,basis_g,kcal,protein_g,carbs_g,fat_g,aliases\n\
             牛めし（並盛）,松屋,serving,350,733,25,90,28,牛丼並盛\n",
        );
        let parsed = parse("t.csv", &bytes).unwrap();

        let report = commit(
            &env.ctx,
            parsed,
            ImportTarget::New {
                name: "松屋菜单".into(),
            },
        )
        .await
        .unwrap();

        assert_eq!(report.imported, 0);
        assert_eq!(report.imported_dishes, 1);
        assert_eq!(report.source.dish_count, 1);

        let chain = sources::SourceChain::load(&env.ctx).await.unwrap();
        let dish = chain
            .lookup_dish("松屋", "牛丼並盛")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(dish.name, "牛めし（並盛）");
        assert_eq!(dish.serving_g, Some(350.0));
    }

    #[tokio::test]
    async fn replace_carries_over_dish_use_count_by_brand_and_name() {
        let env = env().await;
        let bytes1 = csv_bytes(
            "name,brand,basis,kcal,protein_g,carbs_g,fat_g\n牛めし（並盛）,松屋,serving,733,25,90,28\n",
        );
        let report1 = commit(
            &env.ctx,
            parse("t.csv", &bytes1).unwrap(),
            ImportTarget::New {
                name: "松屋菜单".into(),
            },
        )
        .await
        .unwrap();
        let source_id = report1.source.id.clone();

        let chain = sources::SourceChain::load(&env.ctx).await.unwrap();
        let dish = chain
            .lookup_dish("松屋", "牛めし（並盛）")
            .await
            .unwrap()
            .unwrap();
        chain.record_dish_use(&dish).await.unwrap();
        chain.record_dish_use(&dish).await.unwrap();

        let bytes2 = csv_bytes(
            "name,brand,basis,kcal,protein_g,carbs_g,fat_g\n牛めし（並盛）,松屋,serving,700,24,85,26\n",
        );
        commit(
            &env.ctx,
            parse("t.csv", &bytes2).unwrap(),
            ImportTarget::Replace { source_id },
        )
        .await
        .unwrap();

        let chain = sources::SourceChain::load(&env.ctx).await.unwrap();
        let dish = chain
            .lookup_dish("松屋", "牛めし（並盛）")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(dish.use_count, 2);
        assert_eq!(dish.per_serving.kcal, 700.0);
    }

    #[tokio::test]
    async fn export_then_import_round_trips_foods_and_dishes_for_both_formats() {
        let env = env().await;
        let bytes = csv_bytes(
            "name,brand,basis,basis_g,kcal,protein_g,carbs_g,fat_g,fiber_g,aliases\n\
             熟米饭,,100g,,116,2.6,25.9,0.3,0.3,米饭\n\
             牛めし（並盛）,松屋,serving,350,733,25,90,28,3,牛丼並盛\n\
             家常菜,,serving,,600,20,80,15,5,\n",
        );
        let report = commit(
            &env.ctx,
            parse("t.csv", &bytes).unwrap(),
            ImportTarget::New {
                name: "测试集".into(),
            },
        )
        .await
        .unwrap();
        let source_id = report.source.id.clone();

        for format in [ImportFormat::Csv, ImportFormat::Json] {
            let exported = sources::export_source(&env.ctx, &source_id, format)
                .await
                .unwrap();
            let file_name = match format {
                ImportFormat::Csv => "roundtrip.csv",
                ImportFormat::Json => "roundtrip.json",
            };
            let reparsed = parse(file_name, &exported).unwrap();

            assert_eq!(reparsed.valid_food_rows(), 1);
            assert_eq!(reparsed.valid_dish_rows(), 2);
            assert_eq!(reparsed.food_rows[0].macros.kcal, 116.0);
            assert_eq!(reparsed.food_rows[0].aliases, vec![normalize("米饭")]);

            let matched = reparsed
                .dish_rows
                .iter()
                .find(|r| r.name == "牛めし（並盛）")
                .unwrap();
            assert_eq!(matched.brand, "松屋");
            assert_eq!(matched.serving_g, Some(350.0));
            assert_eq!(matched.macros.kcal, 733.0);
            assert_eq!(matched.aliases, vec![normalize("牛丼並盛")]);

            let brandless = reparsed
                .dish_rows
                .iter()
                .find(|r| r.name == "家常菜")
                .unwrap();
            assert_eq!(brandless.brand, "");
            assert_eq!(brandless.serving_g, None);
        }
    }
}
