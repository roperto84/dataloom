//! Importing CSV, JSON and Excel files into a table.
//!
//! Parsing is separate from execution: [`load_source`] produces a table-shaped
//! [`ImportSource`], the dialog builds an [`ImportPlan`] (target table, column
//! mapping, options), and [`spawn_import`] runs the batches on its own
//! connection in a transaction so the UI stays responsive and a failure leaves
//! the table untouched.

use crate::db::{self, Driver, SavedConnection};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SourceFormat {
    Csv,
    Json,
    Excel,
}

impl SourceFormat {
    pub fn label(self) -> &'static str {
        match self {
            SourceFormat::Csv => "CSV / TSV",
            SourceFormat::Json => "JSON",
            SourceFormat::Excel => "Excel",
        }
    }
}

/// Rows and columns read from the file, before any mapping.
#[derive(Clone, Debug)]
pub struct ImportSource {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    /// Sheet names for Excel workbooks.
    pub sheets: Vec<String>,
    /// Non-fatal problems (skipped cells, truncated files…).
    pub warnings: Vec<String>,
    /// Whether the first row was treated as the header.
    pub has_header: bool,
    /// Delimiter used for CSV (`,` for the other formats).
    pub delimiter: char,
}

impl ImportSource {
    /// A few values from one column, used in the mapping table.
    pub fn sample(&self, column: usize, count: usize) -> String {
        let mut out = Vec::new();
        for row in self.rows.iter().take(40) {
            if let Some(Some(value)) = row.get(column) {
                if !value.is_empty() {
                    out.push(value.chars().take(40).collect::<String>());
                }
                if out.len() >= count {
                    break;
                }
            }
        }
        out.join(" · ")
    }
}

pub fn format_for_path(path: &Path) -> Option<SourceFormat> {
    match path
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .as_deref()
    {
        Some("csv" | "tsv" | "txt") => Some(SourceFormat::Csv),
        Some("json") => Some(SourceFormat::Json),
        Some("xlsx" | "xls" | "xlsm" | "ods") => Some(SourceFormat::Excel),
        _ => None,
    }
}

/// Reads a file with default options (comma/tab sniffing, header detection).
pub fn load_source(path: &Path) -> Result<ImportSource, String> {
    let format = format_for_path(path)
        .ok_or_else(|| "Choose a .csv, .tsv, .json, .xlsx or .xls file.".to_string())?;
    match format {
        SourceFormat::Csv => {
            let text = std::fs::read(path)
                .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
            let text = decode_text(&text);
            let delimiter = sniff_delimiter(&text);
            let mut source = parse_csv(&text, delimiter)?;
            source.sheets = vec![delimiter_name(delimiter).to_string()];
            Ok(source)
        }
        SourceFormat::Json => {
            let text = std::fs::read_to_string(path)
                .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
            parse_json(&text)
        }
        SourceFormat::Excel => load_excel(path, 0),
    }
}

fn decode_text(bytes: &[u8]) -> String {
    match String::from_utf8(bytes.to_vec()) {
        Ok(text) => text,
        Err(_) => String::from_utf8_lossy(bytes).to_string(),
    }
}

pub fn delimiter_name(delimiter: char) -> &'static str {
    match delimiter {
        '\t' => "Tab",
        ';' => "Semicolon",
        '|' => "Pipe",
        _ => "Comma",
    }
}

/// Picks the delimiter that appears most often outside quotes in the first
/// few lines. Ties prefer commas, then tabs.
pub fn sniff_delimiter(text: &str) -> char {
    let mut best = (',', 0usize);
    for candidate in [',', '\t', ';', '|'] {
        let mut count = 0usize;
        let mut quoted = false;
        for (index, c) in text.chars().enumerate() {
            if index > 4096 {
                break;
            }
            if c == '"' {
                quoted = !quoted;
            } else if c == candidate && !quoted {
                count += 1;
            }
        }
        if count > best.1 {
            best = (candidate, count);
        }
    }
    best.0
}

/// RFC 4180-ish reader: quoted fields, doubled quotes, newlines inside quotes
/// and CRLF. Fully empty trailing lines are dropped.
pub fn read_records(text: &str, delimiter: char) -> Vec<Vec<String>> {
    let mut records = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' if field.is_empty() => quoted = true,
            c if c == delimiter => {
                record.push(std::mem::take(&mut field));
            }
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            '\n' => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    records.retain(|row| row.iter().any(|cell| !cell.trim().is_empty()));
    records
}

/// Reads and parses a CSV file with the given options.
pub fn parse_csv_file(
    path: &Path,
    delimiter: char,
    has_header: Option<bool>,
) -> Result<ImportSource, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    parse_csv_opts(&decode_text(&bytes), delimiter, has_header)
}

/// Parses delimited text. `has_header` is detected when not forced.
pub fn parse_csv(text: &str, delimiter: char) -> Result<ImportSource, String> {
    parse_csv_opts(text, delimiter, None)
}

/// `has_header = None` keeps the automatic detection.
pub fn parse_csv_opts(
    text: &str,
    delimiter: char,
    has_header: Option<bool>,
) -> Result<ImportSource, String> {
    let records = read_records(text, delimiter);
    if records.is_empty() {
        return Err("The file has no rows.".to_string());
    }
    let header = has_header.unwrap_or_else(|| detect_header(&records));
    Ok(source_from_records(records, header, Vec::new(), delimiter))
}

fn source_from_records(
    records: Vec<Vec<String>>,
    has_header: bool,
    warnings: Vec<String>,
    delimiter: char,
) -> ImportSource {
    let width = records.iter().map(Vec::len).max().unwrap_or(0);
    let columns = if has_header {
        let raw = &records[0];
        normalize_headers(raw, width)
    } else {
        (1..=width).map(|i| format!("column_{i}")).collect()
    };
    let data = if has_header {
        records[1..].to_vec()
    } else {
        records
    };
    let rows = data
        .into_iter()
        .map(|mut row| {
            row.resize(width, String::new());
            row.into_iter()
                .map(|cell| {
                    if cell.is_empty() {
                        None
                    } else {
                        Some(cell)
                    }
                })
                .collect()
        })
        .collect();
    ImportSource {
        columns,
        rows,
        sheets: Vec::new(),
        warnings,
        has_header,
        delimiter,
    }
}

fn detect_header(records: &[Vec<String>]) -> bool {
    let Some(first) = records.first() else {
        return false;
    };
    if first.is_empty() || first.iter().any(|cell| cell.trim().is_empty()) {
        return false;
    }
    let mut seen = std::collections::HashSet::new();
    for cell in first {
        let key = cell.trim().to_ascii_lowercase();
        if !seen.insert(key) {
            return false;
        }
    }
    // A first row with a numeric cell is data ("1,A"), not a header.
    first
        .iter()
        .all(|cell| cell.trim().parse::<f64>().is_err())
}

fn normalize_headers(raw: &[String], width: usize) -> Vec<String> {
    let mut used = std::collections::HashSet::new();
    (0..width)
        .map(|index| {
            let base = raw.get(index).map(|s| s.trim()).unwrap_or_default();
            let base = if base.is_empty() {
                format!("column_{}", index + 1)
            } else {
                base.to_string()
            };
            let mut candidate = base.clone();
            let mut suffix = 2;
            while !used.insert(candidate.to_ascii_lowercase()) {
                candidate = format!("{base}_{suffix}");
                suffix += 1;
            }
            candidate
        })
        .collect()
}

pub fn parse_json(text: &str) -> Result<ImportSource, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("Invalid JSON: {error}"))?;
    let value = match value {
        serde_json::Value::Object(ref map) => {
            // Accept `{"data": [...]}` wrappers with a single array property.
            let arrays: Vec<&serde_json::Value> = map
                .values()
                .filter(|value| value.is_array())
                .collect();
            if map.len() == 1 && arrays.len() == 1 {
                arrays[0].clone()
            } else {
                value
            }
        }
        other => other,
    };
    let (columns, rows) = match value {
        serde_json::Value::Array(items) => {
            if items.is_empty() {
                return Err("The JSON array is empty.".to_string());
            }
            let objects: Vec<&serde_json::Map<String, serde_json::Value>> =
                items.iter().filter_map(|item| item.as_object()).collect();
            if objects.len() == items.len() {
                let mut columns: Vec<String> = Vec::new();
                for object in &objects {
                    for key in object.keys() {
                        if !columns.contains(key) {
                            columns.push(key.clone());
                        }
                    }
                }
                let rows = objects
                    .iter()
                    .map(|object| {
                        columns
                            .iter()
                            .map(|column| object.get(column).and_then(json_cell))
                            .collect()
                    })
                    .collect();
                (columns, rows)
            } else {
                let width = items
                    .iter()
                    .filter_map(|item| item.as_array().map(Vec::len))
                    .max()
                    .unwrap_or(0);
                let columns = (1..=width).map(|i| format!("column_{i}")).collect();
                let rows = items
                    .iter()
                    .map(|item| match item.as_array() {
                        Some(values) => values.iter().map(json_cell).collect(),
                        None => vec![json_cell(item)],
                    })
                    .collect();
                (columns, rows)
            }
        }
        serde_json::Value::Object(_) => {
            let object = value.as_object().expect("checked above");
            let columns: Vec<String> = object.keys().cloned().collect();
            let row = columns.iter().map(|column| json_cell(&object[column])).collect();
            (columns, vec![row])
        }
        _ => return Err("The JSON must be an object or an array of objects.".to_string()),
    };
    let columns = normalize_headers(&columns, columns.len());
    Ok(ImportSource {
        columns,
        rows,
        sheets: Vec::new(),
        warnings: Vec::new(),
        has_header: true,
        delimiter: ',',
    })
}

fn json_cell(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Bool(flag) => Some(flag.to_string()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        other => serde_json::to_string(other).ok(),
    }
}

pub fn load_excel(path: &Path, sheet_index: usize) -> Result<ImportSource, String> {
    load_excel_opts(path, sheet_index, None)
}

/// `has_header = None` keeps the automatic detection.
pub fn load_excel_opts(
    path: &Path,
    sheet_index: usize,
    has_header: Option<bool>,
) -> Result<ImportSource, String> {
    use calamine::{Data, Reader};
    let mut workbook = calamine::open_workbook_auto(path)
        .map_err(|error| format!("Could not open {}: {error}", path.display()))?;
    let sheets = workbook.sheet_names().to_vec();
    let range = workbook
        .worksheet_range_at(sheet_index)
        .ok_or_else(|| "That sheet does not exist.".to_string())?
        .map_err(|error| format!("Could not read the sheet: {error}"))?;
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut warnings = Vec::new();
    let mut dropped = 0usize;
    for row in range.rows() {
        let mut record: Vec<String> = Vec::with_capacity(row.len());
        for cell in row {
            match cell {
                Data::Empty => record.push(String::new()),
                Data::String(text) => record.push(text.clone()),
                Data::Float(value) => record.push(format_excel_number(*value)),
                Data::Int(value) => record.push(value.to_string()),
                Data::Bool(flag) => record.push(flag.to_string()),
                Data::DateTime(datetime) => record.push(excel_datetime(datetime.as_f64())),
                Data::DateTimeIso(text) => record.push(text.clone()),
                Data::DurationIso(text) => record.push(text.clone()),
                Data::Error(_) => {
                    dropped += 1;
                    record.push(String::new());
                }
            }
        }
        if record.iter().any(|cell| !cell.trim().is_empty()) {
            records.push(record);
        }
    }
    if dropped > 0 {
        warnings.push(format!("{dropped} error cell(s) were left empty."));
    }
    if records.is_empty() {
        return Err("The first sheet has no data.".to_string());
    }
    let header = has_header.unwrap_or_else(|| detect_header(&records));
    let mut source = source_from_records(records, header, warnings, ',');
    source.sheets = sheets;
    Ok(source)
}

fn format_excel_number(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{value:.0}")
    } else {
        format!("{value}")
    }
}

/// Excel serial dates count days from 1899-12-30 (accounting for the 1900 leap
/// bug for modern dates). Reuses the app's civil-date helper.
pub fn excel_datetime(serial: f64) -> String {
    let secs = ((serial - 25_569.0) * 86_400.0).round() as i64;
    let (year, month, day, hour, minute, second) = db::utc_parts(secs);
    if hour == 0 && minute == 0 && second == 0 {
        format!("{year:04}-{month:02}-{day:02}")
    } else {
        format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
    }
}

/// A column of the target table (or of the table that will be created).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetColumn {
    pub name: String,
    /// Declared type, used to decide whether literals are quoted.
    pub decl_type: String,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ImportMode {
    NewTable,
    Existing,
}

/// Everything the worker needs. Rows are already projected onto the mapped
/// columns and NULL-normalized, so the worker only builds SQL.
#[derive(Clone, Debug)]
pub struct ImportPlan {
    pub table_sql: String,
    pub table_label: String,
    pub create_sql: Option<String>,
    /// Target columns in insert order.
    pub columns: Vec<TargetColumn>,
    pub rows: Vec<Vec<Option<String>>>,
    pub truncate: bool,
    pub batch_size: usize,
}

pub enum ImportProgress {
    Progress { done: usize, total: usize },
    Finished { rows: usize, elapsed_ms: u128, table: String },
    Failed(String),
    Cancelled,
}

pub struct ImportJob {
    pub rx: Receiver<ImportProgress>,
    pub cancel: Arc<AtomicBool>,
    pub total: usize,
    pub done: usize,
    pub started: Instant,
    pub table_label: String,
}

pub fn spawn_import(saved: SavedConnection, plan: ImportPlan) -> ImportJob {
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let total = plan.rows.len();
    let table_label = plan.table_label.clone();
    let _ = std::thread::Builder::new()
        .name("dataloom-import".to_string())
        .spawn(move || {
            run_import(saved, plan, worker_cancel, tx);
        });
    ImportJob {
        rx,
        cancel,
        total,
        done: 0,
        started: Instant::now(),
        table_label,
    }
}

fn run_import(
    saved: SavedConnection,
    plan: ImportPlan,
    cancel: Arc<AtomicBool>,
    tx: Sender<ImportProgress>,
) {
    let started = Instant::now();
    let driver = saved.driver_kind();
    let mut db = match db::Db::open(&saved) {
        Ok(db) => db,
        Err(error) => {
            let _ = tx.send(ImportProgress::Failed(error));
            return;
        }
    };
    let execute = |db: &mut db::Db, sql: &str| -> Result<(), String> {
        let outcome = db.run_sql(sql, 0);
        outcome.error.map(|error| error).map_or(Ok(()), Err)
    };
    if let Some(create) = &plan.create_sql {
        if let Err(error) = execute(&mut db, create) {
            let _ = tx.send(ImportProgress::Failed(format!(
                "Could not create the table: {error}"
            )));
            return;
        }
    }
    if let Err(error) = execute(&mut db, "BEGIN") {
        let _ = tx.send(ImportProgress::Failed(error));
        return;
    }
    if plan.truncate {
        let sql = format!("DELETE FROM {}", plan.table_sql);
        if let Err(error) = execute(&mut db, &sql) {
            let _ = execute(&mut db, "ROLLBACK");
            let _ = tx.send(ImportProgress::Failed(error));
            return;
        }
    }
    let mut done = 0usize;
    let mut index = 0usize;
    while index < plan.rows.len() {
        if cancel.load(Ordering::SeqCst) {
            let _ = execute(&mut db, "ROLLBACK");
            let _ = tx.send(ImportProgress::Cancelled);
            return;
        }
        let end = (index + plan.batch_size.max(1)).min(plan.rows.len());
        let sql = insert_statement(&plan.table_sql, &plan.columns, &plan.rows[index..end], driver);
        if let Err(error) = execute(&mut db, &sql) {
            let _ = execute(&mut db, "ROLLBACK");
            let _ = tx.send(ImportProgress::Failed(format!(
                "Rows {}–{} failed: {error}",
                index + 1,
                end
            )));
            return;
        }
        done = end;
        let _ = tx.send(ImportProgress::Progress {
            done,
            total: plan.rows.len(),
        });
        index = end;
    }
    if let Err(error) = execute(&mut db, "COMMIT") {
        let _ = execute(&mut db, "ROLLBACK");
        let _ = tx.send(ImportProgress::Failed(error));
        return;
    }
    let _ = tx.send(ImportProgress::Finished {
        rows: done,
        elapsed_ms: started.elapsed().as_millis(),
        table: plan.table_label,
    });
}

/// Quotes a (possibly schema-qualified) table name for the dialect.
pub fn quote_table(driver: Driver, table: &db::TableId) -> String {
    let name = |part: &str| quote_column(driver, part);
    match &table.schema {
        Some(schema) if !schema.is_empty() => {
            if driver == Driver::MySql {
                format!("{}.{}", name(schema), name(&table.name))
            } else {
                format!("{}.{}", name(schema), name(&table.name))
            }
        }
        _ => name(&table.name),
    }
}

pub fn quote_column(driver: Driver, name: &str) -> String {
    match driver {
        Driver::MySql => format!("`{}`", name.replace('`', "``")),
        _ => db::quote_ident(name),
    }
}

/// Builds one multi-row `INSERT` statement. SQL NULL stays unquoted; numeric
/// and boolean targets are written as literals so PostgreSQL can cast them.
pub fn insert_statement(
    table_sql: &str,
    columns: &[TargetColumn],
    rows: &[Vec<Option<String>>],
    driver: Driver,
) -> String {
    let column_list = columns
        .iter()
        .map(|column| quote_column(driver, &column.name))
        .collect::<Vec<_>>()
        .join(", ");
    let mut sql = format!("INSERT INTO {table_sql} ({column_list}) VALUES ");
    for (row_index, row) in rows.iter().enumerate() {
        if row_index > 0 {
            sql.push(',');
        }
        sql.push('(');
        for (index, column) in columns.iter().enumerate() {
            if index > 0 {
                sql.push(',');
            }
            let value = row.get(index).and_then(|value| value.as_deref());
            sql.push_str(&literal_for(value, &column.decl_type, driver));
        }
        sql.push(')');
    }
    sql
}

fn literal_for(value: Option<&str>, decl_type: &str, driver: Driver) -> String {
    let Some(value) = value else {
        return "NULL".to_string();
    };
    let trimmed = value.trim();
    // SQLite columns with no declared type have BLOB affinity; writing bare
    // numbers keeps them numeric instead of turning them into text.
    if decl_type.trim().is_empty()
        && (trimmed.parse::<i64>().is_ok() || trimmed.parse::<f64>().is_ok())
    {
        return trimmed.to_string();
    }
    let kind = db::FieldEditor::from_decl(decl_type, &[]);
    match kind {
        db::FieldEditor::Bool
        | db::FieldEditor::Date
        | db::FieldEditor::DateTime
        | db::FieldEditor::Json
        | db::FieldEditor::Blob
        | db::FieldEditor::Enum(_)
        | db::FieldEditor::Text => {
            let numeric = matches!(kind, db::FieldEditor::Text | db::FieldEditor::Enum(_))
                && is_numeric_type(decl_type);
            if numeric {
                if trimmed.parse::<i64>().is_ok() || trimmed.parse::<f64>().is_ok() {
                    return trimmed.to_string();
                }
            }
            if matches!(kind, db::FieldEditor::Bool) {
                let lower = trimmed.to_ascii_lowercase();
                if matches!(lower.as_str(), "true" | "t" | "1" | "yes") {
                    return match driver {
                        Driver::Sqlite => "1".to_string(),
                        _ => "TRUE".to_string(),
                    };
                }
                if matches!(lower.as_str(), "false" | "f" | "0" | "no") {
                    return match driver {
                        Driver::Sqlite => "0".to_string(),
                        _ => "FALSE".to_string(),
                    };
                }
            }
            sql_string(value, driver)
        }
    }
}

fn is_numeric_type(decl_type: &str) -> bool {
    let decl = decl_type.to_ascii_lowercase();
    decl.contains("int")
        || decl.contains("real")
        || decl.contains("float")
        || decl.contains("double")
        || decl.contains("decimal")
        || decl.contains("numeric")
}

pub fn sql_string(value: &str, driver: Driver) -> String {
    let escaped = match driver {
        Driver::MySql => value.replace('\\', "\\\\").replace('\'', "''"),
        _ => value.replace('\'', "''"),
    };
    format!("'{escaped}'")
}

/// SQL type for a new table, inferred from the sample rows.
pub fn infer_type(values: impl Iterator<Item = Option<String>>, driver: Driver) -> &'static str {
    let mut count = 0usize;
    let mut integers = 0usize;
    let mut floats = 0usize;
    let mut booleans = 0usize;
    for value in values {
        let Some(value) = value else {
            continue;
        };
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        count += 1;
        if trimmed.parse::<i64>().is_ok() {
            integers += 1;
        }
        if trimmed.parse::<f64>().is_ok() {
            floats += 1;
        }
        if matches!(trimmed.to_ascii_lowercase().as_str(), "true" | "false") {
            booleans += 1;
        }
    }
    if count == 0 {
        return "TEXT";
    }
    if booleans == count {
        return match driver {
            Driver::Postgres => "BOOLEAN",
            Driver::MySql => "TINYINT(1)",
            Driver::Sqlite => "INTEGER",
        };
    }
    if integers == count {
        return "INTEGER";
    }
    if floats == count {
        return match driver {
            Driver::Postgres => "DOUBLE PRECISION",
            Driver::MySql => "DOUBLE",
            Driver::Sqlite => "REAL",
        };
    }
    "TEXT"
}

/// Cleans a source header into a usable column name.
pub fn sanitize_column(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() || c == '_' {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() {
        "column".to_string()
    } else if out.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        format!("c_{out}")
    } else {
        out
    }
}

/// Sanitizes and de-duplicates the target column names for a new table.
pub fn new_table_columns(columns: &[String]) -> Vec<String> {
    let mut used = std::collections::HashSet::new();
    columns
        .iter()
        .map(|column| {
            let base = sanitize_column(column);
            let mut candidate = base.clone();
            let mut suffix = 2;
            while !used.insert(candidate.to_ascii_lowercase()) {
                candidate = format!("{base}_{suffix}");
                suffix += 1;
            }
            candidate
        })
        .collect()
}

pub fn create_table_statement(
    driver: Driver,
    table: &str,
    columns: &[TargetColumn],
) -> String {
    let defs = columns
        .iter()
        .map(|column| format!("{} {}", quote_column(driver, &column.name), column.decl_type))
        .collect::<Vec<_>>()
        .join(", ");
    let qualified = match driver {
        Driver::MySql => format!("`{}`", table.replace('`', "``")),
        _ => db::quote_ident(table),
    };
    format!("CREATE TABLE {qualified} ({defs})")
}

/// Opens a native panel for choosing a data file.
pub fn pick_data_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Import data")
        .add_filter("Data files", &["csv", "tsv", "json", "xlsx", "xls", "ods"])
        .add_filter("CSV", &["csv", "tsv", "txt"])
        .add_filter("JSON", &["json"])
        .add_filter("Excel", &["xlsx", "xls", "xlsm", "ods"])
        .pick_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_handles_quotes_delimiters_and_newlines() {
        let text = "name,note,value\r\n\"A, B\",\"line1\nline2\",3\r\nC,\"say \"\"hi\"\"\",4\r\n";
        let source = parse_csv(text, ',').unwrap();
        assert_eq!(source.columns, vec!["name", "note", "value"]);
        assert_eq!(source.rows.len(), 2);
        assert_eq!(source.rows[0][0].as_deref(), Some("A, B"));
        assert_eq!(source.rows[0][1].as_deref(), Some("line1\nline2"));
        assert_eq!(source.rows[1][1].as_deref(), Some("say \"hi\""));
    }

    #[test]
    fn delimiter_and_header_detection() {
        assert_eq!(sniff_delimiter("a;b;c\n1;2;3"), ';');
        assert_eq!(sniff_delimiter("a\tb\tc\n1\t2\t3"), '\t');
        assert_eq!(sniff_delimiter("a,b,c\n1,2,3"), ',');
        assert_eq!(sniff_delimiter("no separators"), ',');

        let header = parse_csv("id,name\n1,A", ',').unwrap();
        assert_eq!(header.columns, vec!["id", "name"]);
        let data = parse_csv("1,A\n2,B", ',').unwrap();
        assert_eq!(data.columns, vec!["column_1", "column_2"]);
        assert_eq!(data.rows.len(), 2);
    }

    #[test]
    fn json_shapes_become_tables() {
        let source =
            parse_json(r#"[{"id":1,"name":"A"},{"id":2,"nick":"B"}]"#).unwrap();
        assert_eq!(source.columns, vec!["id", "name", "nick"]);
        assert_eq!(source.rows[0][2], None);
        assert_eq!(source.rows[1][2].as_deref(), Some("B"));

        let nested = parse_json(r#"[{"id":1,"tags":["a","b"],"meta":{"x":1}}]"#).unwrap();
        assert_eq!(nested.rows[0][1].as_deref(), Some("[\"a\",\"b\"]"));
        assert_eq!(nested.rows[0][2].as_deref(), Some("{\"x\":1}"));

        let wrapped = parse_json(r#"{"data":[{"a":1}]}"#).unwrap();
        assert_eq!(wrapped.columns, vec!["a"]);
        assert!(parse_json("not json").is_err());
        assert!(parse_json("42").is_err());
    }

    #[test]
    fn excel_roundtrip_through_the_export_writer() {
        let dir = std::env::temp_dir().join(format!("dataloom-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sheet.xlsx");
        let bytes = crate::xlsx::workbook(
            "Data",
            &["id".to_string(), "name".to_string()],
            &[
                vec![Some("1".to_string()), Some("Avery".to_string())],
                vec![Some("2".to_string()), Some("Mila".to_string())],
            ],
        );
        std::fs::write(&path, bytes).unwrap();
        let source = load_excel(&path, 0).unwrap();
        assert_eq!(source.columns, vec!["id", "name"]);
        assert_eq!(source.rows.len(), 2);
        assert_eq!(source.rows[0][1].as_deref(), Some("Avery"));
        assert_eq!(source.sheets, vec!["Data"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn excel_datetime_uses_the_1900_epoch() {
        // 2024-01-01 is serial 45292 in Excel.
        assert_eq!(excel_datetime(45_292.0), "2024-01-01");
        assert_eq!(excel_datetime(45_292.5), "2024-01-01 12:00:00");
    }

    #[test]
    fn inserts_quote_by_target_type_and_dialect() {
        let columns = vec![
            TargetColumn {
                name: "id".to_string(),
                decl_type: "INTEGER".to_string(),
            },
            TargetColumn {
                name: "name".to_string(),
                decl_type: "TEXT".to_string(),
            },
            TargetColumn {
                name: "ok".to_string(),
                decl_type: "BOOLEAN".to_string(),
            },
        ];
        let rows = vec![
            vec![Some("1".to_string()), Some("O'Brien".to_string()), None],
            vec![Some("2".to_string()), Some("\\path".to_string()), Some("true".to_string())],
        ];
        let sql = insert_statement("`t`", &columns, &rows, Driver::MySql);
        assert!(sql.starts_with("INSERT INTO `t` (`id`, `name`, `ok`) VALUES (1,'O''Brien',NULL)"));
        assert!(sql.contains("'\\\\path'"), "{sql}");
        assert!(sql.contains(",TRUE)"));

        let sqlite = insert_statement("\"t\"", &columns, &rows, Driver::Sqlite);
        assert!(sqlite.contains(",1)"), "{sqlite}");
        assert!(sqlite.contains("'\\path'") || sqlite.contains("'\\\\path'"), "{sqlite}");
    }

    fn saved_sqlite(path: &Path) -> SavedConnection {
        SavedConnection {
            id: 1,
            name: "Import test".to_string(),
            path: path.to_string_lossy().to_string(),
            driver: "SQLite".to_string(),
            host: String::new(),
            port: 0,
            user: String::new(),
            password: String::new(),
            database: String::new(),
            read_only: false,
            ssl_mode: "disable".to_string(),
            ssl_root_cert: String::new(),
            ssh_host: String::new(),
            ssh_port: 0,
            ssh_user: String::new(),
            ssh_key_path: String::new(),
            group_id: None,
        }
    }

    fn wait(job: &ImportJob) -> ImportProgress {
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        loop {
            match job
                .rx
                .recv_timeout(std::time::Duration::from_millis(200))
            {
                Ok(ImportProgress::Progress { .. }) => continue,
                Ok(other) => return other,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if Instant::now() > deadline {
                        panic!("import did not finish");
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("import worker vanished")
                }
            }
        }
    }

    #[test]
    fn worker_imports_existing_table_and_rolls_back_bad_batches() {
        let dir = std::env::temp_dir().join(format!("dataloom-import-run-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("run.db");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT, active BOOLEAN);",
            )
            .unwrap();
        }
        let columns = vec![
            TargetColumn {
                name: "id".to_string(),
                decl_type: "INTEGER".to_string(),
            },
            TargetColumn {
                name: "name".to_string(),
                decl_type: "TEXT".to_string(),
            },
            TargetColumn {
                name: "active".to_string(),
                decl_type: "BOOLEAN".to_string(),
            },
        ];
        let plan = ImportPlan {
            table_sql: "\"people\"".to_string(),
            table_label: "people".to_string(),
            create_sql: None,
            columns: columns.clone(),
            rows: vec![
                vec![Some("1".to_string()), Some("Avery".to_string()), Some("true".to_string())],
                vec![Some("2".to_string()), Some("O'Brien".to_string()), None],
            ],
            truncate: false,
            batch_size: 1,
        };
        let job = spawn_import(saved_sqlite(&path), plan);
        match wait(&job) {
            ImportProgress::Finished { rows, .. } => assert_eq!(rows, 2),
            other => panic!("unexpected import result: {:?}", std::mem::discriminant(&other)),
        }
        let conn = rusqlite::Connection::open(&path).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM people", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2);
        let name: String = conn
            .query_row("SELECT name FROM people WHERE id = 2", [], |row| row.get(0))
            .unwrap();
        assert_eq!(name, "O'Brien");
        let active: Option<i64> = conn
            .query_row("SELECT active FROM people WHERE id = 2", [], |row| row.get(0))
            .unwrap();
        assert_eq!(active, None);

        // A failing batch must leave the table untouched.
        let bad = ImportPlan {
            table_sql: "\"people\"".to_string(),
            table_label: "people".to_string(),
            create_sql: None,
            columns,
            rows: vec![vec![
                Some("not-a-number".to_string()),
                Some("bad".to_string()),
                None,
            ]],
            truncate: true,
            batch_size: 10,
        };
        let job = spawn_import(saved_sqlite(&path), bad);
        assert!(matches!(wait(&job), ImportProgress::Failed(_)));
        let count: i64 = conn
            .query_row("SELECT count(*) FROM people", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 2, "the failed import must roll back the DELETE");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn worker_creates_a_new_table() {
        let dir = std::env::temp_dir().join(format!("dataloom-import-new-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("new.db");
        rusqlite::Connection::open(&path).unwrap();
        let columns = vec![
            TargetColumn {
                name: "id".to_string(),
                decl_type: "INTEGER".to_string(),
            },
            TargetColumn {
                name: "label".to_string(),
                decl_type: "TEXT".to_string(),
            },
        ];
        let plan = ImportPlan {
            table_sql: "\"imported\"".to_string(),
            table_label: "imported".to_string(),
            create_sql: Some(create_table_statement(Driver::Sqlite, "imported", &columns)),
            rows: vec![vec![Some("1".to_string()), Some("one".to_string())]],
            columns,
            truncate: false,
            batch_size: 10,
        };
        let job = spawn_import(saved_sqlite(&path), plan);
        assert!(matches!(wait(&job), ImportProgress::Finished { rows: 1, .. }));
        let conn = rusqlite::Connection::open(&path).unwrap();
        let label: String = conn
            .query_row("SELECT label FROM imported WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(label, "one");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn type_inference_and_column_sanitizing() {
        assert_eq!(
            infer_type(
                [Some("1".to_string()), Some("2".to_string())].into_iter(),
                Driver::Postgres
            ),
            "INTEGER"
        );
        assert_eq!(
            infer_type(
                [Some("1.5".to_string()), Some("2".to_string())].into_iter(),
                Driver::Postgres
            ),
            "DOUBLE PRECISION"
        );
        assert_eq!(
            infer_type(
                [Some("true".to_string()), Some("false".to_string())].into_iter(),
                Driver::Sqlite
            ),
            "INTEGER"
        );
        assert_eq!(
            infer_type([Some("x".to_string())].into_iter(), Driver::Sqlite),
            "TEXT"
        );

        assert_eq!(sanitize_column("First Name"), "First_Name");
        assert_eq!(sanitize_column("2024 total"), "c_2024_total");
        assert_eq!(sanitize_column("  "), "column");
        assert_eq!(
            new_table_columns(&["a b".to_string(), "a_b".to_string()]),
            vec!["a_b", "a_b_2"]
        );
    }
}
