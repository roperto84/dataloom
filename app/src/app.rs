use crate::db::{self, Driver, SavedConnection, Store, TableId, TableInfo};
use crate::theme::{self, Palette};
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Margin, Rect, RichText, Rounding, ScrollArea,
    Sense, Stroke, TextEdit, Ui, Vec2,
};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

// TablePlus saved `NSSplitView Subview Frames WorkspaceSplitView` records the
// sidebar at 296.5 pt; 297 keeps the same layout in egui.
const SIDEBAR_W: f32 = 297.0;

// `RightSidebarController.nib`: the row detail cells are 300 pt wide
// (`{{0, 37}, {300, 44}}` and similar frames), so the panel's content width
// follows that value. The panel itself is resizable like the original's
// `NSSplitView` right sidebar.
pub(crate) const ROW_DETAIL_W: f32 = 300.0;

// `RightSidebarController.nib`: the split view's row-detail pane has a 240 pt
// minimum width (width constraint 240 at priority 750), so the recreation never
// shrinks below that.
pub(crate) const ROW_DETAIL_MIN_W: f32 = 240.0;

pub(crate) const ROW_LIMIT_OPTIONS: &[(usize, &str)] = &[
    (0, "No limit"),
    (100, "100 rows"),
    (500, "500 rows"),
    (1_000, "1,000 rows"),
    (5_000, "5,000 rows"),
    (10_000, "10,000 rows"),
    (50_000, "50,000 rows"),
    (100_000, "100,000 rows"),
    (500_000, "500,000 rows"),
];

pub(crate) fn row_limit_label(limit: usize) -> String {
    ROW_LIMIT_OPTIONS
        .iter()
        .find(|(value, _)| *value == limit)
        .map(|(_, label)| label.to_string())
        .unwrap_or_else(|| limit.to_string())
}

pub struct ScreenshotJob {
    pub path: PathBuf,
    pub requested: bool,
}

pub struct DataloomApp {
    pub(crate) store: Store,
    pub(crate) live: Option<Live>,
    pub(crate) tabs: Vec<Tab>,
    pub(crate) active: usize,
    pub(crate) theme_dark: bool,
    pub(crate) palette: Palette,
    pub(crate) sidebar: SidebarState,
    pub(crate) new_connection: Option<NewConnection>,
    pub(crate) row_editor: Option<RowEditor>,
    pub(crate) column_editor: Option<ColumnEditor>,
    pub(crate) column_delete: Option<ColumnDelete>,
    pub(crate) row_delete: Option<RowDelete>,
    pub(crate) connection_delete: Option<ConnectionDelete>,
    pub(crate) clone_table: Option<CloneTable>,
    pub(crate) group_editor: Option<GroupEditor>,
    pub(crate) group_delete: Option<GroupDelete>,
    pub(crate) truncate_table: Option<TruncateTable>,
    pub(crate) delete_table: Option<DeleteTable>,
    pub(crate) db_tools: Option<DbTools>,
    pub(crate) toast: Option<Toast>,
    pub(crate) screenshot: Option<ScreenshotJob>,
    pub(crate) frame_count: u64,
    pub(crate) query_counter: usize,
    pub(crate) tab_counter: u64,
    pub(crate) job_counter: u64,
    /// Worker thread and channels that execute SQL away from the UI thread.
    pub(crate) query_worker: Option<QueryWorker>,
    /// The query currently running, if any.
    pub(crate) running_query: Option<RunningQuery>,
    pub(crate) welcome_error: Option<String>,
    pub(crate) pending_copy: Option<String>,
    pub(crate) query_history: Vec<String>,
    pub(crate) history_open: bool,
    pub(crate) connection_search: String,
    pub(crate) about_open: bool,
    /// Preferences restored from `settings.json`.
    pub(crate) settings: crate::settings::Settings,
    /// Native macOS menu bar; `None` before the first frame or off macOS.
    pub(crate) native_menu: Option<crate::menu::NativeMenu>,
    pub(crate) palette_open: bool,
    pub(crate) palette_search: String,
    pub(crate) settings_open: bool,
    pub(crate) close_confirm: bool,
    /// Set by Cmd+F; the active view focuses its search field and clears it.
    pub(crate) focus_search: bool,
    /// Most recent cell edit, used by Undo/Redo.
    pub(crate) edit_history: EditHistory,
    /// Modal for editing JSON/blob values that do not fit a grid cell.
    pub(crate) value_editor: Option<ValueEditor>,
    /// Parameter values captured before running a query with `:name` tokens.
    pub(crate) params_dialog: Option<ParamsDialog>,
    pub(crate) last_session_save: Instant,
    pub(crate) palette_index: usize,
    /// Set once the user confirmed closing with unsaved SQL.
    pub(crate) allow_close: bool,
    /// Guards against the native menu accelerator and the egui shortcut both
    /// firing for the same key press.
    pub(crate) last_command: Option<(crate::menu::CommandId, Instant)>,
    /// Last session written to disk, used to skip unchanged rewrites.
    pub(crate) last_session: Option<crate::settings::Session>,
    /// True while reopening the saved session, so connect does not auto-run.
    pub(crate) restoring_session: bool,
    /// Data import dialog (CSV/Excel/JSON with column mapping).
    pub(crate) import_dialog: Option<ImportDialog>,
    /// Running import worker, if any.
    pub(crate) import_job: Option<crate::import::ImportJob>,
    /// Dialog kept so a failed import can be retried without remapping.
    pub(crate) import_retry: Option<ImportDialog>,
    /// Transaction open on the SQL editor's connection, if any.
    pub(crate) editor_tx: Option<EditorTransaction>,
    /// Write statement waiting for the safe-mode confirmation.
    pub(crate) safe_query: Option<SafeQuery>,
}

/// Undo/redo journal for cell and row-detail edits. Entries survive tab
/// switches because they address the row by table + key, not by tab index.
#[derive(Default)]
pub struct EditHistory {
    pub undo: Vec<EditRecord>,
    pub redo: Vec<EditRecord>,
}

pub struct EditRecord {
    pub table: TableId,
    pub key_column: String,
    pub key_value: String,
    pub column: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

pub struct ValueEditor {
    pub tab_id: u64,
    pub table: TableId,
    pub key_column: String,
    pub key_value: String,
    pub column: String,
    pub kind: crate::db::FieldEditor,
    pub text: String,
    /// Value loaded when the window opened, recorded in the undo journal.
    pub before: Option<String>,
    pub error: Option<String>,
}

/// State of the data import dialog. The parsed rows live here until the user
/// confirms the mapping, then they move to the worker.
#[derive(Clone)]
pub struct ImportDialog {
    pub path: PathBuf,
    pub format: crate::import::SourceFormat,
    pub source: crate::import::ImportSource,
    pub sheet: usize,
    pub delimiter: char,
    pub mode: crate::import::ImportMode,
    pub new_table: String,
    pub existing: Option<TableId>,
    /// Target columns of the current mode.
    pub targets: Vec<crate::import::TargetColumn>,
    /// One entry per source column: the target column index or `None` to skip.
    pub mapping: Vec<Option<usize>>,
    pub null_tokens: String,
    pub empty_as_null: bool,
    pub trim: bool,
    pub truncate: bool,
    pub batch_size: usize,
    pub error: Option<String>,
}

/// A write statement paused for the safe-mode confirmation.
pub struct SafeQuery {
    pub tab_id: u64,
    pub sql: String,
    pub max_rows: usize,
}

pub struct ParamsDialog {
    pub tab_id: u64,
    pub names: Vec<String>,
    pub values: Vec<String>,
}

pub struct Live {
    pub saved: SavedConnection,
    pub db: db::Db,
    pub tables: Vec<TableInfo>,
    pub views: Vec<TableInfo>,
    /// Kept alive for the lifetime of the connection when SSH is used.
    pub tunnel: Option<crate::tunnel::SshTunnel>,
    /// Column names per table, filled lazily for SQL completion.
    pub column_cache: std::collections::HashMap<String, Vec<String>>,
}

pub struct SidebarState {
    pub search: String,
    pub tables_open: bool,
    pub views_open: bool,
}

pub struct Tab {
    /// Stable identity used to route an async result back to its tab even if
    /// other tabs are opened or closed while the query runs.
    pub id: u64,
    pub title: String,
    pub kind: TabKind,
}

/// What the worker should do with a request. Transaction commands use the
/// worker's own connection so the editor statements share one transaction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QueryKind {
    Sql,
    Begin,
    Commit,
    Rollback,
}

impl QueryKind {
    pub fn is_transaction(self) -> bool {
        self != QueryKind::Sql
    }
}

/// Request sent to the background query thread.
pub struct QueryRequest {
    /// Identifies the request so a stale result can be ignored.
    pub id: u64,
    pub sql: String,
    pub max_rows: usize,
    /// The connection's current read-only flag, applied before running so a
    /// toolbar toggle reaches the worker without reconnecting.
    pub read_only: bool,
    /// The worker sends its cancellation handle here before running.
    pub control: Sender<db::QueryCancel>,
    /// Set by the UI if Cancel is pressed before the worker starts, so the
    /// worker can skip a query that has not entered its blocking phase yet.
    pub cancelled: Arc<AtomicBool>,
}

/// The background query thread. It owns a dedicated `Db` connection so session
/// state (temporary tables, variables) survives across queries, while the UI
/// thread keeps its own connection for metadata and cell edits.
pub struct QueryWorker {
    pub tx: Sender<QueryRequest>,
    pub result_rx: Receiver<(u64, db::QueryOutcome)>,
}

impl QueryWorker {
    fn spawn(saved: SavedConnection) -> QueryWorker {
        let (request_tx, request_rx) = mpsc::channel::<QueryRequest>();
        let (result_tx, result_rx) = mpsc::channel::<(u64, db::QueryOutcome)>();
        let _ = thread::Builder::new()
            .name("dataloom-query".to_string())
            .spawn(move || {
                let mut db = db::Db::open(&saved);
                while let Ok(request) = request_rx.recv() {
                    let outcome = match db.as_mut() {
                        Ok(db) => {
                            db.set_read_only(request.read_only);
                            let cancel = db.query_cancel_handle(&saved);
                            let _ = request.control.send(cancel);
                            if request.cancelled.load(Ordering::SeqCst) {
                                db::QueryOutcome {
                                    error: Some("Query cancelled.".to_string()),
                                    ..db::QueryOutcome::default()
                                }
                            } else {
                                db.run_sql(&request.sql, request.max_rows)
                            }
                        }
                        Err(error) => db::QueryOutcome {
                            error: Some(error.clone()),
                            ..db::QueryOutcome::default()
                        },
                    };
                    if result_tx.send((request.id, outcome)).is_err() {
                        break;
                    }
                }
            });
        QueryWorker {
            tx: request_tx,
            result_rx,
        }
    }
}

/// An open transaction on the query worker's connection.
pub struct EditorTransaction {
    pub started: Instant,
    /// Statements run inside the transaction.
    pub statements: usize,
}

/// State of the query running in the worker thread.
pub struct RunningQuery {
    pub id: u64,
    pub kind: QueryKind,
    pub tab_id: u64,
    pub started: Instant,
    pub cancelled: bool,
    pub cancel_flag: Arc<AtomicBool>,
    /// Handle sent by the worker before it starts blocking.
    pub cancel: Option<db::QueryCancel>,
    pub control_rx: Receiver<db::QueryCancel>,
}

pub enum TabKind {
    Query(QueryTab),
    Table(TableTab),
}

pub struct QueryTab {
    pub sql: String,
    pub outcome: Option<db::QueryOutcome>,
    /// A background query for this tab is in flight.
    pub running: bool,
    pub mode: ResultMode,
    pub selected: crate::widgets::GridSelection,
    /// Header sort for the in-memory result rows.
    pub sort: Vec<db::SortSpec>,
    /// False after the header changes so the next frame re-sorts the rows.
    pub sorted: bool,
    /// Rows as returned by the query, kept only while a local header sort is
    /// active so clearing the sort can restore the original order.
    pub original_rows: Option<Vec<Vec<Option<String>>>>,
    pub row_limit: usize,
    /// The SQL changed since it was last run; closing the tab asks first.
    pub dirty: bool,
    /// Fraction of the tab height occupied by the editor (dragged by the user).
    pub split: f32,
    /// Line reported by the last error, used for the gutter marker.
    pub error_line: Option<usize>,
    /// In-memory result search (Cmd+F).
    pub search: String,
    /// Caret position, kept so templates insert where the user is typing.
    pub cursor: usize,
    /// Ask the editor to move the caret to `cursor` on the next frame.
    pub move_cursor: bool,
    pub completion_index: usize,
    pub completion_word: String,
    /// Last search text, used to reset the selection when the filter changes.
    pub search_last: String,
    /// User-resized column widths, in column order.
    pub widths: Vec<f32>,
    /// Vertical scroll offset, persisted in the session.
    pub scroll: f32,
    /// One-shot scroll position applied on the first frame after restore.
    pub restore_scroll: Option<f32>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ResultMode {
    Result,
    Messages,
}

pub struct TableTab {
    pub table: db::TableId,
    pub mode: TableMode,
    pub offset: usize,
    pub limit: usize,
    pub paging_open: bool,
    pub limit_text: String,
    pub offset_text: String,
    pub paging_error: Option<String>,
    pub filter: db::TableFilter,
    pub applied_filter: db::TableFilter,
    pub filters_open: bool,
    pub data: db::TablePage,
    pub structure: Vec<db::ColumnMeta>,
    pub indexes: Vec<db::IndexMeta>,
    pub ddl: Option<String>,
    pub selected: crate::widgets::GridSelection,
    /// Server-side ORDER BY terms chosen in the column headers.
    pub sort: Vec<db::SortSpec>,
    /// User-resized column widths, in column order.
    pub widths: Vec<f32>,
    /// Vertical scroll offset, persisted in the session.
    pub scroll: f32,
    /// One-shot scroll position applied on the first frame after restore.
    pub restore_scroll: Option<f32>,
    /// In-cell editor state; `None` when no cell is being edited.
    pub edit: Option<crate::widgets::GridEdit>,
    /// The right-hand row detail panel is open (TablePlus: "Toggle Row Detail
    /// (Space)" in WorkspaceWindow.nib). It starts open like the original's
    /// right sidebar.
    pub detail_open: bool,
    pub detail_search: String,
    pub detail_match: DetailMatch,
    pub detail_case: bool,
    /// Vertical form of the selected record; `None` until a row is selected.
    pub detail: Option<RowDetail>,
}

/// Matching strategy for the row detail field search. The original's nib lists
/// "Fuzzy - default" and "Contains" (RowDetailSearchStrategy / matching menu).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DetailMatch {
    Fuzzy,
    Contains,
}

/// One field of the selected record, shown vertically in the row detail panel.
/// Mirrors the original's `RowDetailTextCell`, `RowDetailByteaCell` and
/// `RowDetailDropdownCell` (RightSidebarController.nib).
pub struct RowDetailField {
    pub name: String,
    /// Raw declared type from the table structure (`varchar(255)`, `TEXT`…).
    pub decl_type: String,
    /// TablePlus display name for the type: long, var_string, blob, boolean…
    pub type_label: String,
    pub primary_key: bool,
    pub nullable: bool,
    /// `true` when the stored value is SQL NULL (shown as a gray "NULL").
    pub is_null: bool,
    /// The value as loaded; `None` is SQL NULL.
    pub original: Option<String>,
    /// Closed set of values for enum columns.
    pub options: Vec<String>,
    /// Current editor text. Empty while `is_null` and untouched.
    pub value: String,
}

impl RowDetailField {
    pub fn editor_kind(&self) -> crate::db::FieldEditor {
        crate::db::FieldEditor::from_decl(&self.decl_type, &self.options)
    }

    /// Text/blob values open in the auto-sized multi-line editor
    /// (`RowDetailByteaCell` → `RowDetailTextView`). Short values stay
    /// single-line so Enter saves them; NULL blobs keep the compact row,
    /// exactly like the reference screenshot.
    pub fn is_multiline(&self) -> bool {
        if self.is_null {
            return false;
        }
        let t = self.decl_type.to_ascii_lowercase();
        let lob = t.contains("text")
            || t.contains("blob")
            || t.contains("clob")
            || t.contains("json")
            || t.contains("xml");
        self.value.contains('\n') || (lob && self.value.chars().count() > 72)
    }
}

/// The selected record rendered vertically in the right-hand panel.
pub struct RowDetail {
    /// Grid row index the fields were loaded from.
    pub row: usize,
    pub key_column: String,
    pub key_value: String,
    pub fields: Vec<RowDetailField>,
    pub error: Option<String>,
}

impl TableTab {
    /// Builds the vertical form for one loaded row. `None` disables editing
    /// (views and keyless tables), matching `TablePage::key_column`.
    pub(crate) fn build_row_detail(&self, row: usize, driver: Driver) -> Option<RowDetail> {
        let values = self.data.rows.get(row)?;
        let key_column = self.data.key_column.clone().unwrap_or_default();
        let key_value = self
            .data
            .rowids
            .get(row)
            .cloned()
            .flatten()
            .unwrap_or_default();
        let fields = self
            .data
            .columns
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let meta = self.structure.iter().find(|c| &c.name == name);
                let original = values.get(i).cloned().flatten();
                let decl_type = meta.map(|m| m.decl_type.clone()).unwrap_or_default();
                RowDetailField {
                    name: name.clone(),
                    type_label: tableplus_type_label(&decl_type, driver),
                    decl_type,
                    primary_key: meta.map(|m| m.primary_key).unwrap_or(false),
                    nullable: meta.map(|m| !(m.not_null || m.primary_key)).unwrap_or(true),
                    is_null: original.is_none(),
                    value: original.clone().unwrap_or_default(),
                    original,
                    options: meta.map(|m| m.options.clone()).unwrap_or_default(),
                }
            })
            .collect();
        Some(RowDetail {
            row,
            key_column,
            key_value,
            fields,
            error: None,
        })
    }
}

/// The display names TablePlus uses in the row detail cells: integers are
/// "long", character columns "var_string", text/blob columns "blob". SQLite's
/// TEXT is the normal string type (there is no separate varchar), so it stays
/// "var_string" there; MySQL/PostgreSQL TEXT is the blob-ish category.
fn tableplus_type_label(decl: &str, driver: Driver) -> String {
    let t = decl.to_ascii_lowercase();
    let sqlite = matches!(driver, Driver::Sqlite);
    if t.is_empty() {
        String::new()
    } else if t.contains("int") || t.contains("serial") {
        "long".into()
    } else if t.contains("char") || t.contains("string") || t.contains("uuid") {
        "var_string".into()
    } else if t.contains("blob")
        || t.contains("clob")
        || t.contains("json")
        || t.contains("xml")
    {
        "blob".into()
    } else if t.contains("text") {
        if sqlite {
            "var_string".into()
        } else {
            "blob".into()
        }
    } else if t.contains("bool") {
        "boolean".into()
    } else if t.contains("timestamp") || t.contains("datetime") {
        "datetime".into()
    } else if t.contains("date") {
        "date".into()
    } else if t.contains("time") {
        "time".into()
    } else if t.contains("double") || t.contains("real") || t.contains("float") {
        "double".into()
    } else if t.contains("decimal") || t.contains("numeric") || t.contains("money") {
        "decimal".into()
    } else {
        t
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TableMode {
    Data,
    Structure,
}

pub struct NewConnection {
    pub driver: Driver,
    pub name: String,
    pub path: String,
    pub path_touched: bool,
    pub host: String,
    pub port: String,
    pub user: String,
    pub password: String,
    pub database: String,
    pub read_only: bool,
    pub ssl_mode: String,
    pub ssl_root_cert: String,
    pub ssh_enabled: bool,
    pub ssh_host: String,
    pub ssh_port: String,
    pub ssh_user: String,
    pub ssh_key_path: String,
    /// Folder selected in the connection form; `None` is ungrouped.
    pub group_id: Option<u64>,
    pub error: Option<String>,
    pub editing: Option<u64>,
}

pub struct RowField {
    pub name: String,
    pub decl_type: String,
    pub primary_key: bool,
    pub value: String,
}

pub struct RowEditor {
    pub tab: usize,
    pub table: TableId,
    pub fields: Vec<RowField>,
    pub error: Option<String>,
}

pub struct ColumnEditor {
    pub tab: usize,
    pub table: TableId,
    /// `None` when adding a column; `Some` when editing an existing one.
    pub original: Option<String>,
    pub name: String,
    pub decl_type: String,
    pub not_null: bool,
    pub default_value: String,
    pub original_type: String,
    pub original_not_null: bool,
    pub error: Option<String>,
}

pub struct ColumnDelete {
    pub tab: usize,
    pub table: TableId,
    pub column: String,
    pub error: Option<String>,
}

pub struct RowDelete {
    pub tab: usize,
    pub table: TableId,
    pub key_column: String,
    pub key_value: String,
    pub row_number: usize,
    pub error: Option<String>,
}

pub struct ConnectionDelete {
    pub id: u64,
    pub name: String,
}

/// "Clone table" dialog: copies structure and rows into a new name.
pub struct CloneTable {
    pub source: TableId,
    pub name: String,
    pub error: Option<String>,
}

/// "Truncate table" confirmation dialog.
pub struct TruncateTable {
    pub table: TableId,
    pub error: Option<String>,
}

/// "Delete table" confirmation dialog.
pub struct DeleteTable {
    pub table: TableId,
    pub error: Option<String>,
}

/// Name-editing dialog for a connection group.
pub struct GroupEditor {
    /// `None` creates a group; `Some` renames that group.
    pub id: Option<u64>,
    pub name: String,
    pub error: Option<String>,
}

/// Confirmation dialog for deleting a connection group.
pub struct GroupDelete {
    pub id: u64,
    pub name: String,
}

/// One action picked from a connection's right-click menu.
pub enum ConnectionMenuAction {
    Edit(SavedConnection),
    Duplicate(SavedConnection),
    Delete(SavedConnection),
    Export(SavedConnection),
}

/// One action picked from a table's sidebar context menu.
pub enum TableMenuAction {
    ExportSql(TableId),
    ExportCsv(TableId),
    ExportXlsx(TableId),
    ImportSql(TableId),
    Clone(TableId),
    Truncate(TableId),
    Delete(TableId),
}

/// File format used by the table export menu.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExportFormat {
    Sql,
    Csv,
    Xlsx,
}

/// Picks a table-menu action and dismisses the whole context menu. A leaf item
/// in a submenu only closes that submenu by itself, so the shared context-menu
/// state is cleared as well (id string from egui's `menu` module).
fn pick_table_menu_action(ui: &mut Ui, out: &mut Option<TableMenuAction>, action: TableMenuAction) {
    *out = Some(action);
    crate::widgets::close_context_menu(ui);
}

/// Contents of a table's right-click menu. Shared by the context menu so the
/// sidebar and any future entry points stay in sync.
fn table_menu_contents(
    ui: &mut Ui,
    table: &TableId,
    read_only: bool,
    out: &mut Option<TableMenuAction>,
) {
    ui.set_min_width(196.0);
    ui.menu_button("Export  >", |ui| {
        if ui.button("SQL (.sql)…").clicked() {
            pick_table_menu_action(ui, out, TableMenuAction::ExportSql(table.clone()));
        }
        if ui.button("Excel (.xlsx)…").clicked() {
            pick_table_menu_action(ui, out, TableMenuAction::ExportXlsx(table.clone()));
        }
        if ui.button("CSV (.csv)…").clicked() {
            pick_table_menu_action(ui, out, TableMenuAction::ExportCsv(table.clone()));
        }
    });
    if ui
        .add_enabled(!read_only, egui::Button::new("Import (SQL)…"))
        .on_disabled_hover_text("Read-only mode is on")
        .clicked()
    {
        pick_table_menu_action(ui, out, TableMenuAction::ImportSql(table.clone()));
    }
    ui.separator();
    if ui
        .add_enabled(!read_only, egui::Button::new("Clone…"))
        .on_disabled_hover_text("Read-only mode is on")
        .clicked()
    {
        pick_table_menu_action(ui, out, TableMenuAction::Clone(table.clone()));
    }
    if ui
        .add_enabled(!read_only, egui::Button::new("Truncate…"))
        .on_disabled_hover_text("Read-only mode is on")
        .clicked()
    {
        pick_table_menu_action(ui, out, TableMenuAction::Truncate(table.clone()));
    }
    if ui
        .add_enabled(!read_only, egui::Button::new("Delete…"))
        .on_disabled_hover_text("Read-only mode is on")
        .clicked()
    {
        pick_table_menu_action(ui, out, TableMenuAction::Delete(table.clone()));
    }
}

/// Sections of the database tools window.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToolsTab {
    Export,
    Import,
    Backups,
}

/// State of the "Database tools" window: SQL export, SQL import and backups.
pub struct DbTools {
    pub tab: ToolsTab,
    /// Table of the active tab, when it is a table tab.
    pub table: Option<TableId>,
    /// Export only the active table instead of the whole database.
    pub scope_table: bool,
    /// Filter captured when the tools opened, so the export can match the
    /// table view. `None` when the table had no active filter.
    pub filter: Option<db::TableFilter>,
    /// Whether `filter` is applied to the table export.
    pub use_filter: bool,
    pub export_path: String,
    pub import_path: String,
    pub message: Option<String>,
    pub message_is_error: bool,
    pub backups: Vec<db::BackupFile>,
    /// Backup waiting for a second click before it replaces the database.
    pub confirm_restore: Option<PathBuf>,
}

#[derive(Clone, Copy)]
pub enum ToastKind {
    Info,
    Error,
}

pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub until: Instant,
}

impl DataloomApp {
    pub fn new(cc: &eframe::CreationContext<'_>, screenshot_path: Option<PathBuf>) -> Self {
        // Dataloom opens in the light theme on every launch, even when macOS is
        // in Dark Mode (the original follows the system via `ThemeID` =
        // "builtin:auto"). The toolbar toggle switches to dark.
        let settings = crate::settings::Settings::load();
        cc.egui_ctx.set_theme(if settings.dark {
            egui::ThemePreference::Dark
        } else {
            egui::ThemePreference::Light
        });
        let initial_dark = settings.dark;
        let palette = theme::apply(
            &cc.egui_ctx,
            initial_dark,
            settings.data_font_size,
            settings.sql_font_size,
        );
        let mut store = Store::load();

        // Seed the sample connection on the very first launch only. An empty
        // store on a later launch means the user deleted their connections on
        // purpose, so the sample must not come back.
        if db::should_seed_sample(&store, db::store_exists()) {
            if let Ok(path) = db::ensure_sample_database() {
                store.add("Sample shop", path.to_str().unwrap_or("sample-shop.db"));
            }
        }

        // Every launch starts on the connections screen: no last connection is
        // reopened and no tabs are restored. Connecting is an explicit choice.
        let mut app = Self {
            store,
            live: None,
            tabs: Vec::new(),
            active: 0,
            theme_dark: initial_dark,
            palette,
            sidebar: SidebarState {
                search: String::new(),
                tables_open: true,
                views_open: true,
            },
            new_connection: None,
            row_editor: None,
            column_editor: None,
            column_delete: None,
            row_delete: None,
            connection_delete: None,
            clone_table: None,
            group_editor: None,
            group_delete: None,
            truncate_table: None,
            delete_table: None,
            db_tools: None,
            toast: None,
            screenshot: screenshot_path.map(|path| ScreenshotJob {
                path,
                requested: false,
            }),
            frame_count: 0,
            query_counter: 0,
            tab_counter: 0,
            job_counter: 0,
            query_worker: None,
            running_query: None,
            welcome_error: None,
            pending_copy: None,
            query_history: db::load_query_history(),
            history_open: false,
            connection_search: String::new(),
            about_open: false,
            settings,
            native_menu: None,
            palette_open: false,
            palette_search: String::new(),
            settings_open: false,
            close_confirm: false,
            focus_search: false,
            edit_history: EditHistory::default(),
            value_editor: None,
            params_dialog: None,
            last_session_save: Instant::now(),
            palette_index: 0,
            allow_close: false,
            last_command: None,
            last_session: None,
            restoring_session: false,
            import_dialog: None,
            import_job: None,
            import_retry: None,
            editor_tx: None,
            safe_query: None,
        };
        if app.screenshot.is_some() {
            app.open_screenshot_workspace();
        } else if app.settings.restore_session {
            app.restore_session();
        }
        app
    }

    /// `--screenshot` is a development aid: open a SQLite connection and its
    /// first table so the capture shows a real data grid instead of the
    /// connections screen. Normal launches are untouched.
    fn open_screenshot_workspace(&mut self) {
        let saved = self
            .store
            .connections
            .iter()
            .find(|c| c.driver == Driver::Sqlite.label())
            .cloned();
        if let Some(saved) = saved {
            if self.connect_saved(saved).is_err() {
                return;
            }
        } else {
            // A store with only remote connections has no SQLite entry to
            // show; use the bundled sample so the capture still has a grid.
            self.open_sample();
            if self.live.is_none() {
                return;
            }
        }
        let table = self
            .live
            .as_ref()
            .and_then(|live| live.tables.first().map(TableInfo::id));
        if let Some(table) = table {
            self.open_table_tab(table);
        }
        // The capture should show the new row detail panel populated, not its
        // empty state, so select the first record like a user would.
        let driver = self
            .live
            .as_ref()
            .map(|live| live.db.driver)
            .unwrap_or(Driver::Sqlite);
        if let Some(tab) = self.tabs.get_mut(self.active) {
            if let TabKind::Table(tt) = &mut tab.kind {
                if !tt.data.rows.is_empty() {
                    tt.selected.set_cell(0, 0);
                    tt.detail = tt.build_row_detail(0, driver);
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // Connection lifecycle
    // ------------------------------------------------------------------

    pub(crate) fn connect_saved(&mut self, saved: SavedConnection) -> Result<(), String> {
        // Drop the old query worker and stop anything still running against
        // the previous connection.
        self.cancel_query();
        self.clear_running_state();
        self.editor_tx = None;
        self.safe_query = None;
        // The previous connection's focused row detail edit is saved against
        // the old handle before it is replaced.
        self.flush_row_detail_edits(self.active);
        // Passwords live in the Keychain once the store has been saved; a
        // legacy in-memory password is used as-is.
        let mut credentials = saved.clone();
        if credentials.password.is_empty() && credentials.id != 0 {
            if let Some(password) = db::load_password(credentials.id) {
                credentials.password = password;
            }
        }
        // Start the tunnel before opening the database. The tunnel owns the
        // ssh child process and is kept in `Live`.
        let (tunnel, effective) = if credentials.driver_kind() != Driver::Sqlite
            && !credentials.ssh_host.trim().is_empty()
        {
            let (tunnel, effective) = crate::tunnel::start(&credentials)?;
            (Some(tunnel), effective)
        } else {
            (None, credentials.clone())
        };
        let db = db::Db::open(&effective)?;
        // Only drop the old query worker once the new connection opened: if
        // the new one fails, the current workspace keeps working.
        self.query_worker = None;
        let all = db.list_tables().unwrap_or_default();
        let (tables, views): (Vec<_>, Vec<_>) = all.into_iter().partition(|t| !t.is_view);
        let has_sample_data = tables.iter().any(|t| t.name == "customers");
        let mut display = saved.clone();
        display.password.clear();
        self.live = Some(Live {
            saved: display,
            db,
            tables,
            views,
            tunnel,
            column_cache: std::collections::HashMap::new(),
        });
        // A dedicated connection keeps query session state across runs and
        // lets the UI cancel a blocking statement.
        self.query_worker = Some(QueryWorker::spawn(effective));
        self.store.last_id = Some(saved.id);
        self.store.save();
        self.welcome_error = None;
        self.tabs.clear();
        self.active = 0;
        self.db_tools = None;
        self.query_counter = 0;
        self.new_query_tab(true);
        if has_sample_data && !self.restoring_session {
            self.run_active();
        }
        if self.screenshot.is_none() {
            self.push_toast(format!("Connected to {}", saved.name), ToastKind::Info);
        }
        Ok(())
    }

    /// Names offered by the editor's completion popup: tables, views and the
    /// columns of up to 25 tables, cached for the lifetime of the connection.
    pub(crate) fn completion_sources(&mut self) -> Vec<String> {
        let Some(live) = &mut self.live else {
            return Vec::new();
        };
        if live.column_cache.is_empty() {
            let tables: Vec<TableId> = live
                .tables
                .iter()
                .take(25)
                .map(TableInfo::id)
                .collect();
            for table in tables {
                let sql = table.sql();
                if let Ok(columns) = live.db.table_structure(&sql) {
                    live.column_cache.insert(
                        sql,
                        columns.into_iter().map(|column| column.name).collect(),
                    );
                }
            }
        }
        let mut out = Vec::new();
        out.extend(live.tables.iter().map(|table| table.name.clone()));
        out.extend(live.views.iter().map(|view| view.name.clone()));
        for columns in live.column_cache.values() {
            out.extend(columns.iter().cloned());
        }
        out.sort_by_key(|name| name.to_ascii_lowercase());
        out.dedup();
        out
    }

    pub(crate) fn refresh_schema(&mut self) {
        if let Some(live) = &mut self.live {
            if let Ok(all) = live.db.list_tables() {
                let (tables, views): (Vec<_>, Vec<_>) = all.into_iter().partition(|t| !t.is_view);
                live.tables = tables;
                live.views = views;
            }
        }
    }

    pub(crate) fn disconnect(&mut self) {
        // Save a row detail field that is still focused before dropping tabs.
        self.flush_row_detail_edits(self.active);
        self.cancel_query();
        self.clear_running_state();
        self.query_worker = None;
        self.editor_tx = None;
        self.safe_query = None;
        self.live = None;
        self.tabs.clear();
        // An explicit disconnect ends the restorable workspace.
        crate::settings::Session::clear();
        self.active = 0;
        self.welcome_error = None;
        self.db_tools = None;
    }

    /// Opens the database tools on the Export tab. The export scope defaults to
    /// the active table when there is one, so dumping a table is one click.
    pub(crate) fn open_db_tools(&mut self) {
        let (table, filter) = match self.tabs.get(self.active).map(|tab| &tab.kind) {
            Some(TabKind::Table(tt)) => (
                Some(tt.table.clone()),
                (!tt.applied_filter.is_empty()).then(|| tt.applied_filter.clone()),
            ),
            _ => (None, None),
        };
        let use_filter = filter.is_some();
        let db_name = self
            .live
            .as_ref()
            .map(|live| live.saved.name.clone())
            .unwrap_or_else(|| "database".to_string());
        let export_path = db::default_dump_path(&db_name).display().to_string();
        let backups = self
            .live
            .as_ref()
            .map(|live| db::list_backups(&live.saved))
            .unwrap_or_default();
        self.db_tools = Some(DbTools {
            tab: ToolsTab::Export,
            scope_table: table.is_some(),
            table,
            filter,
            use_filter,
            export_path,
            import_path: String::new(),
            message: None,
            message_is_error: false,
            backups,
            confirm_restore: None,
        });
    }

    /// Re-reads the schema and every table tab after an imported script, so
    /// new tables and rows show up immediately.
    pub(crate) fn refresh_after_import(&mut self) {
        self.refresh_schema();
        for idx in 0..self.tabs.len() {
            if matches!(self.tabs[idx].kind, TabKind::Table(_)) {
                self.refresh_table_tab(idx);
            }
        }
    }

    /// Runs one action chosen from a table's sidebar context menu. File
    /// dialogs and short operations are handled here; clone/truncate/delete
    /// open their confirmation dialogs.
    pub(crate) fn handle_table_menu(&mut self, action: TableMenuAction) {
        match action {
            TableMenuAction::ExportSql(table) => {
                self.export_table(
                    &table,
                    ExportFormat::Sql,
                    &db::TableFilter::default(),
                    &[],
                );
            }
            TableMenuAction::ExportCsv(table) => {
                self.export_table(
                    &table,
                    ExportFormat::Csv,
                    &db::TableFilter::default(),
                    &[],
                );
            }
            TableMenuAction::ExportXlsx(table) => {
                self.export_table(
                    &table,
                    ExportFormat::Xlsx,
                    &db::TableFilter::default(),
                    &[],
                );
            }
            TableMenuAction::ImportSql(_table) => {
                let Some(path) = pick_sql_file(None) else {
                    return;
                };
                let script = match std::fs::read_to_string(&path) {
                    Ok(script) => script,
                    Err(error) => {
                        self.push_toast(
                            format!("Could not read {}: {error}", path.display()),
                            ToastKind::Error,
                        );
                        return;
                    }
                };
                let result = match &self.live {
                    Some(live) => live.db.import_sql(&script),
                    None => Err("The connection is closed.".to_string()),
                };
                match result {
                    Ok(count) => {
                        self.refresh_after_import();
                        self.push_toast(
                            format!(
                                "Imported {count} statement{} from {}",
                                if count == 1 { "" } else { "s" },
                                path.display()
                            ),
                            ToastKind::Info,
                        );
                    }
                    Err(error) => {
                        // A failed script can still have run earlier
                        // statements; refresh so the sidebar matches reality.
                        self.refresh_after_import();
                        self.push_toast(error, ToastKind::Error);
                    }
                }
            }
            TableMenuAction::Clone(table) => {
                self.clone_table = Some(CloneTable {
                    name: format!("{}_copy", table.name),
                    source: table,
                    error: None,
                });
            }
            TableMenuAction::Truncate(table) => {
                self.truncate_table = Some(TruncateTable {
                    table,
                    error: None,
                });
            }
            TableMenuAction::Delete(table) => {
                self.delete_table = Some(DeleteTable {
                    table,
                    error: None,
                });
            }
        }
    }

    /// Writes an export of one table. `filter` limits the rows to the matching
    /// records (every matching row, not just the visible page); an empty
    /// filter exports the whole table.
    pub(crate) fn export_table(
        &mut self,
        table: &TableId,
        format: ExportFormat,
        filter: &db::TableFilter,
        sort: &[db::SortSpec],
    ) {
        let (extension, file_label, file_type, kind_label) = match format {
            ExportFormat::Sql => ("sql", "SQL files", "sql", "SQL"),
            ExportFormat::Csv => ("csv", "CSV files", "csv", "CSV"),
            ExportFormat::Xlsx => ("xlsx", "Excel workbooks", "xlsx", "Excel workbook"),
        };
        let display = table.display();
        let default = db::export_path(&display, extension);
        let Some(path) = save_export_dialog(&default, "Export table", file_label, &[file_type])
        else {
            return;
        };
        let result = match &self.live {
            Some(live) => {
                let name = table.sql();
                let bytes = match format {
                    ExportFormat::Sql => live
                        .db
                        .dump_table_sql(&name, filter, sort)
                        .map(String::into_bytes),
                    ExportFormat::Csv => live.db.export_table_csv(&name, filter, sort),
                    ExportFormat::Xlsx => live.db.export_table_xlsx(&name, filter, sort),
                };
                bytes.and_then(|bytes| {
                    std::fs::write(&path, bytes)
                        .map_err(|error| format!("Could not write {}: {error}", path.display()))
                })
            }
            None => Err("The connection is closed.".to_string()),
        };
        match result {
            Ok(()) => {
                let scope = if filter.is_empty() {
                    "table"
                } else {
                    "filtered rows"
                };
                self.push_toast(
                    format!("Exported {scope} to {} ({kind_label})", path.display()),
                    ToastKind::Info,
                );
            }
            Err(error) => self.push_toast(error, ToastKind::Error),
        }
    }

    /// Exports the active table tab with its applied filter. The export
    /// contains every matching row, not just the current page.
    pub(crate) fn export_active_table_view(&mut self, format: ExportFormat) {
        let target = match self.tabs.get(self.active).map(|tab| &tab.kind) {
            Some(TabKind::Table(tt)) => Some((
                tt.table.clone(),
                tt.applied_filter.clone(),
                tt.sort.clone(),
            )),
            _ => None,
        };
        if let Some((table, filter, sort)) = target {
            self.export_table(&table, format, &filter, &sort);
        }
    }

    /// Replaces a SQLite database file with a backup. Only offered for whole
    /// file backups: SQL dumps are restored through Import instead.
    pub(crate) fn restore_backup(&mut self, backup: &Path) -> Result<(), String> {
        let Some(live) = &self.live else {
            return Err("The connection is closed.".to_string());
        };
        if live.db.driver != Driver::Sqlite {
            return Err(
                "Only SQLite file backups can be restored here; import the SQL dump instead."
                    .to_string(),
            );
        }
        if live.db.is_read_only() {
            return Err("Read-only mode is on; turn it off to restore a backup.".to_string());
        }
        if backup.extension().map(|ext| ext != "db").unwrap_or(true) {
            return Err("Only .db file backups can be restored here.".to_string());
        }
        let saved = live.saved.clone();
        if saved.path.trim().is_empty() {
            return Err("This connection has no database file to restore into.".to_string());
        }
        // Close the database before replacing the file on disk.
        self.disconnect();
        std::fs::copy(backup, &saved.path)
            .map_err(|e| format!("Could not restore {}: {e}", backup.display()))?;
        self.connect_saved(saved)
    }

    pub(crate) fn open_edit_connection_dialog(&mut self, saved: &SavedConnection) {
        self.new_connection = Some(NewConnection {
            driver: saved.driver_kind(),
            name: saved.name.clone(),
            path: saved.path.clone(),
            path_touched: true,
            host: if saved.host.is_empty() {
                "127.0.0.1".to_string()
            } else {
                saved.host.clone()
            },
            port: if saved.port == 0 {
                String::new()
            } else {
                saved.port.to_string()
            },
            user: saved.user.clone(),
            password: saved.password.clone(),
            database: saved.database.clone(),
            read_only: saved.read_only,
            ssl_mode: match saved.ssl_mode.trim().to_ascii_lowercase().as_str() {
                "require" => "require".to_string(),
                "verify-ca" => "verify-ca".to_string(),
                "verify-full" => "verify-full".to_string(),
                _ => "disable".to_string(),
            },
            ssl_root_cert: saved.ssl_root_cert.clone(),
            ssh_enabled: !saved.ssh_host.trim().is_empty(),
            ssh_host: saved.ssh_host.clone(),
            ssh_port: if saved.ssh_port == 0 {
                "22".to_string()
            } else {
                saved.ssh_port.to_string()
            },
            ssh_user: saved.ssh_user.clone(),
            ssh_key_path: saved.ssh_key_path.clone(),
            group_id: saved.group_id,
            error: None,
            editing: Some(saved.id),
        });
    }

    pub(crate) fn confirm_delete_connection(&mut self, saved: &SavedConnection) {
        self.connection_delete = Some(ConnectionDelete {
            id: saved.id,
            name: saved.name.clone(),
        });
    }

    pub(crate) fn open_group_editor(&mut self, group: Option<(u64, String)>) {
        self.group_editor = Some(GroupEditor {
            id: group.as_ref().map(|(id, _)| *id),
            name: group
                .map(|(_, name)| name)
                .unwrap_or_else(|| "New group".to_string()),
            error: None,
        });
    }

    /// Copies a saved connection under a new name; the duplicate stays in the
    /// same group.
    pub(crate) fn duplicate_connection(&mut self, saved: &SavedConnection) {
        let source_id = saved.id;
        let mut copy = saved.clone();
        copy.id = 0;
        copy.name = available_connection_name(&self.store.connections, &saved.name);
        // The original record usually has no password: it lives in the
        // Keychain under the old id. Copy it so the duplicate can connect.
        if copy.password.is_empty() && source_id != 0 {
            if let Some(password) = db::load_password(source_id) {
                copy.password = password;
            }
        }
        let copy = self.store.add_connection(copy);
        self.push_toast(format!("Duplicated as {}", copy.name), ToastKind::Info);
    }

    /// Writes one connection as JSON. Passwords are never exported; the id and
    /// group are local-only and are cleared.
    pub(crate) fn export_connection(&mut self, saved: &SavedConnection) {
        let default = db::export_path(&saved.name, "json");
        let Some(path) = save_export_dialog(&default, "Export connection", "JSON files", &["json"]) else {
            return;
        };
        let mut exported = saved.clone();
        exported.id = 0;
        exported.group_id = None;
        exported.password.clear();
        let document = serde_json::json!({
            "version": 1,
            "connection": exported,
        });
        let result = serde_json::to_string_pretty(&document)
            .map_err(|error| error.to_string())
            .and_then(|text| {
                std::fs::write(&path, text).map_err(|error| {
                    format!("Could not write {}: {error}", path.display())
                })
            });
        match result {
            Ok(()) => self.push_toast(
                format!("Exported connection to {}", path.display()),
                ToastKind::Info,
            ),
            Err(error) => self.push_toast(error, ToastKind::Error),
        }
    }

    /// Imports connections from a JSON file: a single connection object, an
    /// array of connections, or `{ "connections": [...] }` / `{ "connection": … }`.
    /// Imported ids and local groups are dropped; names are made unique.
    pub(crate) fn import_connections(&mut self) {
        let Some(path) = pick_connections_file() else {
            return;
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) => {
                self.push_toast(
                    format!("Could not read {}: {error}", path.display()),
                    ToastKind::Error,
                );
                return;
            }
        };
        let imported = match parse_connections_json(&text) {
            Ok(connections) => connections,
            Err(error) => {
                self.push_toast(error, ToastKind::Error);
                return;
            }
        };
        let mut existing = self.store.connections.clone();
        let mut prepared = Vec::new();
        for mut connection in imported {
            if connection.name.trim().is_empty() {
                continue;
            }
            connection.id = 0;
            connection.group_id = None;
            connection.name = available_connection_name(&existing, &connection.name);
            existing.push(connection.clone());
            prepared.push(connection);
        }
        let count = prepared.len();
        if !prepared.is_empty() {
            self.store.add_connections(prepared);
        }
        if count == 0 {
            self.push_toast(
                "No connections were found in that file.".to_string(),
                ToastKind::Error,
            );
        } else {
            self.push_toast(
                format!(
                    "Imported {count} connection{}",
                    if count == 1 { "" } else { "s" }
                ),
                ToastKind::Info,
            );
        }
    }

    // ------------------------------------------------------------------
    // Data import (CSV / Excel / JSON)
    // ------------------------------------------------------------------

    /// Opens the import dialog for a file, parsing it and guessing a target.
    pub(crate) fn open_import_dialog(&mut self, path: PathBuf) {
        let Some(format) = crate::import::format_for_path(&path) else {
            self.push_toast(
                "Choose a .csv, .tsv, .json, .xlsx or .xls file.".to_string(),
                ToastKind::Error,
            );
            return;
        };
        if self.live.is_none() {
            self.push_toast("Open a connection first.", ToastKind::Error);
            return;
        }
        if self.is_read_only() {
            self.push_toast(
                "Read-only mode is on: turn it off before importing data.".to_string(),
                ToastKind::Error,
            );
            return;
        }
        let source = match crate::import::load_source(&path) {
            Ok(source) => source,
            Err(error) => {
                self.push_toast(error, ToastKind::Error);
                return;
            }
        };
        let delimiter = source.delimiter;
        let active_table = self.tabs.get(self.active).and_then(|tab| match &tab.kind {
            TabKind::Table(table) => Some(table.table.clone()),
            _ => None,
        });
        let mode = if active_table.is_some() {
            crate::import::ImportMode::Existing
        } else {
            crate::import::ImportMode::NewTable
        };
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_else(|| "imported".to_string());
        let mut dialog = ImportDialog {
            path,
            format,
            sheet: 0,
            delimiter,
            new_table: crate::import::sanitize_column(&stem),
            existing: active_table,
            mode,
            targets: Vec::new(),
            mapping: Vec::new(),
            source,
            null_tokens: "NULL, \\N".to_string(),
            empty_as_null: false,
            trim: true,
            truncate: false,
            batch_size: 100,
            error: None,
        };
        self.refresh_import_targets(&mut dialog);
        self.import_dialog = Some(dialog);
    }

    /// Rebuilds the target column list and a name-first mapping.
    pub(crate) fn refresh_import_targets(&self, dialog: &mut ImportDialog) {
        let driver = self
            .live
            .as_ref()
            .map(|live| live.db.driver)
            .unwrap_or(Driver::Sqlite);
        match dialog.mode {
            crate::import::ImportMode::Existing => {
                if dialog.existing.is_none() {
                    dialog.existing = self
                        .live
                        .as_ref()
                        .and_then(|live| live.tables.first().map(TableInfo::id));
                }
                dialog.targets = match (&self.live, &dialog.existing) {
                    (Some(live), Some(table)) => live
                        .db
                        .table_structure(&table.sql())
                        .unwrap_or_default()
                        .into_iter()
                        .map(|column| crate::import::TargetColumn {
                            name: column.name,
                            decl_type: column.decl_type,
                        })
                        .collect(),
                    _ => Vec::new(),
                };
            }
            crate::import::ImportMode::NewTable => {
                let names = crate::import::new_table_columns(&dialog.source.columns);
                dialog.targets = names
                    .iter()
                    .enumerate()
                    .map(|(index, name)| crate::import::TargetColumn {
                        name: name.clone(),
                        decl_type: crate::import::infer_type(
                            dialog
                                .source
                                .rows
                                .iter()
                                .map(|row| row.get(index).cloned().flatten()),
                            driver,
                        )
                        .to_string(),
                    })
                    .collect();
            }
        }
        dialog.mapping = default_import_mapping(&dialog.targets, &dialog.source.columns);
    }

    /// Re-reads the file after the delimiter, sheet or header choice changes.
    pub(crate) fn reload_import_source(&self, dialog: &mut ImportDialog) {
        let header = Some(dialog.source.has_header);
        let result = match dialog.format {
            crate::import::SourceFormat::Csv => crate::import::parse_csv_file(
                &dialog.path,
                dialog.delimiter,
                header,
            ),
            crate::import::SourceFormat::Json => crate::import::load_source(&dialog.path),
            crate::import::SourceFormat::Excel => {
                crate::import::load_excel_opts(&dialog.path, dialog.sheet, header)
            }
        };
        match result {
            Ok(source) => {
                dialog.source = source;
                dialog.error = None;
                self.refresh_import_targets(dialog);
            }
            Err(error) => dialog.error = Some(error),
        }
    }

    /// Validates the dialog and starts the background import.
    pub(crate) fn begin_import(&mut self, dialog: ImportDialog) {
        let Some(live) = &self.live else {
            self.push_toast("The connection is closed.", ToastKind::Error);
            return;
        };
        let driver = live.db.driver;
        let mut keep: Vec<(usize, usize)> = dialog
            .mapping
            .iter()
            .enumerate()
            .filter_map(|(source, target)| target.map(|target| (target, source)))
            .collect();
        keep.sort_by_key(|(target, _)| *target);
        if keep.is_empty() {
            self.import_dialog = Some(ImportDialog {
                error: Some("Map at least one column.".to_string()),
                ..dialog
            });
            return;
        }
        let mut seen = std::collections::HashSet::new();
        if !keep.iter().all(|(target, _)| seen.insert(*target)) {
            self.import_dialog = Some(ImportDialog {
                error: Some("Two source columns map to the same target.".to_string()),
                ..dialog
            });
            return;
        }
        let columns: Vec<crate::import::TargetColumn> = keep
            .iter()
            .filter_map(|(target, _)| dialog.targets.get(*target).cloned())
            .collect();
        if columns.is_empty() {
            self.import_dialog = Some(ImportDialog {
                error: Some("The target table has no matching columns.".to_string()),
                ..dialog
            });
            return;
        }

        let tokens: Vec<String> = dialog
            .null_tokens
            .split(',')
            .map(|token| token.trim().to_ascii_lowercase())
            .filter(|token| !token.is_empty())
            .collect();
        let trim = dialog.trim;
        let empty_as_null = dialog.empty_as_null;
        let rows: Vec<Vec<Option<String>>> = dialog
            .source
            .rows
            .iter()
            .map(|row| {
                keep.iter()
                    .map(|(_, source)| {
                        let raw = row.get(*source).cloned().flatten();
                        let value = if trim {
                            raw.map(|value| value.trim().to_string())
                        } else {
                            raw
                        };
                        match value {
                            None => None,
                            Some(value) => {
                                let probe = value.trim().to_ascii_lowercase();
                                if (empty_as_null && value.is_empty())
                                    || tokens.iter().any(|token| token == &probe)
                                {
                                    None
                                } else {
                                    Some(value)
                                }
                            }
                        }
                    })
                    .collect()
            })
            .collect();
        let total = rows.len();

        let (table_sql, table_label, create_sql) = match dialog.mode {
            crate::import::ImportMode::Existing => {
                let Some(table) = dialog.existing.clone() else {
                    self.import_dialog = Some(ImportDialog {
                        error: Some("Choose the table to import into.".to_string()),
                        ..dialog
                    });
                    return;
                };
                (
                    crate::import::quote_table(driver, &table),
                    table.display(),
                    None,
                )
            }
            crate::import::ImportMode::NewTable => {
                let name = dialog.new_table.trim().to_string();
                if name.is_empty() {
                    self.import_dialog = Some(ImportDialog {
                        error: Some("Give the new table a name.".to_string()),
                        ..dialog
                    });
                    return;
                }
                let exists = live
                    .tables
                    .iter()
                    .any(|table| table.name.eq_ignore_ascii_case(&name));
                if exists {
                    self.import_dialog = Some(ImportDialog {
                        error: Some(format!(
                            "A table named {name} already exists. Choose Existing table instead."
                        )),
                        ..dialog
                    });
                    return;
                }
                (
                    crate::import::quote_column(driver, &name),
                    name.clone(),
                    Some(crate::import::create_table_statement(driver, &name, &columns)),
                )
            }
        };

        let plan = crate::import::ImportPlan {
            table_sql,
            table_label,
            create_sql,
            columns,
            rows,
            truncate: dialog.truncate && dialog.mode == crate::import::ImportMode::Existing,
            batch_size: dialog.batch_size.clamp(1, 1000),
        };
        let saved = live.saved.clone();
        self.import_retry = Some(dialog.clone());
        self.import_job = Some(crate::import::spawn_import(saved, plan));
        self.import_dialog = None;
        self.push_toast(format!("Importing {total} rows…"), ToastKind::Info);
    }

    pub(crate) fn cancel_import(&mut self) {
        if let Some(job) = &self.import_job {
            job.cancel.store(true, Ordering::SeqCst);
            self.push_toast("Cancelling the import…", ToastKind::Info);
        }
    }

    fn poll_import(&mut self, ctx: &egui::Context) {
        let mut finished: Option<crate::import::ImportProgress> = None;
        let mut disconnected = false;
        if let Some(job) = &mut self.import_job {
            loop {
                match job.rx.try_recv() {
                    Ok(crate::import::ImportProgress::Progress { done, total }) => {
                        job.done = done.min(total);
                    }
                    Ok(other) => {
                        finished = Some(other);
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        if let Some(progress) = finished {
            self.import_job = None;
            match progress {
                crate::import::ImportProgress::Finished {
                    rows,
                    elapsed_ms,
                    table,
                } => {
                    self.import_retry = None;
                    self.refresh_schema();
                    let target = self
                        .live
                        .as_ref()
                        .and_then(|live| {
                            live.tables
                                .iter()
                                .find(|info| info.name == table)
                                .map(TableInfo::id)
                        })
                        .unwrap_or_else(|| TableId::with_schema(None, table.clone()));
                    self.push_toast(
                        format!(
                            "Imported {rows} rows into {table} ({})",
                            crate::widgets::fmt_ms(elapsed_ms)
                        ),
                        ToastKind::Info,
                    );
                    self.open_table_tab(target);
                }
                crate::import::ImportProgress::Failed(error) => {
                    self.push_toast(format!("Import failed: {error}"), ToastKind::Error);
                    // Reopen the dialog with the original mapping and the
                    // error so the user can fix it and retry.
                    if let Some(mut dialog) = self.import_retry.take() {
                        dialog.error = Some(error);
                        self.import_dialog = Some(dialog);
                    }
                }
                crate::import::ImportProgress::Cancelled => {
                    self.import_retry = None;
                    self.push_toast(
                        "Import cancelled; the transaction was rolled back.".to_string(),
                        ToastKind::Info,
                    );
                }
                crate::import::ImportProgress::Progress { .. } => {}
            }
            ctx.request_repaint();
        } else if disconnected {
            self.import_job = None;
            self.push_toast("The import worker stopped unexpectedly.", ToastKind::Error);
            ctx.request_repaint();
        }
    }

    pub(crate) fn open_sample(&mut self) {
        match db::ensure_sample_database() {
            Ok(path) => {
                let path_text = path.to_str().unwrap_or("sample-shop.db").to_string();
                let saved = self
                    .store
                    .connections
                    .iter()
                    .find(|c| c.path == path_text)
                    .cloned()
                    .unwrap_or_else(|| self.store.add("Sample shop", &path_text));
                if let Err(e) = self.connect_saved(saved) {
                    self.welcome_error = Some(e);
                }
            }
            Err(e) => self.welcome_error = Some(e),
        }
    }

    // ------------------------------------------------------------------
    // Tabs
    // ------------------------------------------------------------------

    pub(crate) fn new_query_tab(&mut self, select: bool) {
        let has_sample_data = self
            .live
            .as_ref()
            .map(|live| live.tables.iter().any(|t| t.name == "customers"))
            .unwrap_or(false);
        let sql = match &self.live {
            Some(live) if live.db.driver == Driver::Sqlite && !has_sample_data => {
                db::default_query_inspect()
            }
            Some(live) => live.db.default_query(),
            None => db::default_query(),
        };
        self.push_query_tab(sql, select);
    }

    fn next_tab_id(&mut self) -> u64 {
        self.tab_counter = self.tab_counter.saturating_add(1);
        self.tab_counter
    }

    fn next_job_id(&mut self) -> u64 {
        self.job_counter = self.job_counter.saturating_add(1);
        self.job_counter
    }

    fn push_query_tab(&mut self, sql: String, select: bool) {
        self.query_counter += 1;
        let id = self.next_tab_id();
        let title = if self.query_counter == 1 {
            "Query".to_string()
        } else {
            format!("Query {}", self.query_counter)
        };
        self.tabs.push(Tab {
            id,
            title,
            kind: TabKind::Query(QueryTab {
                sql,
                outcome: None,
                running: false,
                mode: ResultMode::Result,
                selected: crate::widgets::GridSelection::default(),
                sort: Vec::new(),
                sorted: true,
                original_rows: None,
                row_limit: db::DEFAULT_ROW_LIMIT,
                dirty: false,
                split: self.settings.editor_split,
                error_line: None,
                search: String::new(),
                cursor: 0,
                move_cursor: false,
                completion_index: 0,
                completion_word: String::new(),
                search_last: String::new(),
                widths: Vec::new(),
                scroll: 0.0,
                restore_scroll: None,
            }),
        });
        if select {
            self.active = self.tabs.len().saturating_sub(1);
        }
    }

    pub(crate) fn open_table_tab(&mut self, table: TableId) {
        if let Some(i) = self
            .tabs
            .iter()
            .position(|t| matches!(&t.kind, TabKind::Table(tt) if tt.table == table))
        {
            self.active = i;
            return;
        }
        let Some(live) = &self.live else {
            return;
        };
        let sql_name = table.sql();
        let data = live.db.fetch_table(
            &sql_name,
            db::DEFAULT_TABLE_PAGE_LIMIT,
            0,
            &db::TableFilter::default(),
            &[],
        );
        let structure = live.db.table_structure(&sql_name).unwrap_or_default();
        let indexes = live.db.table_indexes(&sql_name);
        let ddl = live.db.table_ddl(&sql_name);
        let id = self.next_tab_id();
        self.tabs.push(Tab {
            id,
            title: table.display(),
            kind: TabKind::Table(TableTab {
                table,
                mode: TableMode::Data,
                offset: 0,
                limit: db::DEFAULT_TABLE_PAGE_LIMIT,
                paging_open: false,
                limit_text: db::DEFAULT_TABLE_PAGE_LIMIT.to_string(),
                offset_text: "0".into(),
                paging_error: None,
                filter: db::TableFilter::default(),
                applied_filter: db::TableFilter::default(),
                filters_open: false,
                data,
                structure,
                indexes,
                ddl,
                selected: crate::widgets::GridSelection::default(),
                sort: Vec::new(),
                widths: Vec::new(),
                scroll: 0.0,
                restore_scroll: None,
                edit: None,
                detail_open: true,
                detail_search: String::new(),
                detail_match: DetailMatch::Fuzzy,
                detail_case: false,
                detail: None,
            }),
        });
        self.active = self.tabs.len().saturating_sub(1);
    }

    pub(crate) fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        let closing_id = self.tabs.get(index).map(|tab| tab.id);
        if self
            .running_query
            .as_ref()
            .map(|job| Some(job.tab_id) == closing_id)
            .unwrap_or(false)
        {
            self.cancel_query();
        }
        // Closing while a row detail field is focused must not drop the edit.
        self.flush_row_detail_edits(index);
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.active = 0;
        } else if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        } else if index < self.active {
            self.active -= 1;
        }
    }

    /// Snapshot of the open tabs, written to `session.json`.
    pub(crate) fn capture_session(&self) -> crate::settings::Session {
        use crate::settings::{Session, SessionTab};
        let (connection_id, connection_name) = match &self.live {
            Some(live) => (Some(live.saved.id), live.saved.name.clone()),
            None => (None, String::new()),
        };
        let tabs = self
            .tabs
            .iter()
            .map(|tab| match &tab.kind {
                TabKind::Query(query) => SessionTab::query(
                    tab.title.clone(),
                    query.sql.clone(),
                    query.row_limit,
                    query.scroll,
                ),
                TabKind::Table(table) => SessionTab::table(
                    &table.table,
                    table.offset,
                    table.limit,
                    &table.applied_filter,
                    &table.sort,
                    table.detail_open,
                    table.scroll,
                ),
            })
            .collect();
        Session {
            connection_id,
            connection_name,
            active: self.active,
            tabs,
        }
    }

    /// Writes the session when the workspace changed. Called from `update` on
    /// a short timer so typing does not hit the disk on every keystroke.
    pub(crate) fn autosave_session(&mut self, force: bool) {
        if !self.settings.restore_session || self.live.is_none() {
            return;
        }
        if self.screenshot.is_some() {
            return;
        }
        if !force && self.last_session_save.elapsed() < Duration::from_secs(2) {
            return;
        }
        let session = self.capture_session();
        if self.last_session.as_ref() == Some(&session) {
            return;
        }
        self.last_session_save = Instant::now();
        session.save();
        self.last_session = Some(session);
    }

    /// Reopens the connection and tabs from the last session, skipping
    /// anything that no longer exists (a deleted connection or table).
    pub(crate) fn restore_session(&mut self) {
        let session = crate::settings::Session::load();
        if session.is_empty() {
            return;
        }
        let Some(saved) = session
            .connection_id
            .and_then(|id| self.store.connections.iter().find(|c| c.id == id).cloned())
        else {
            return;
        };
        self.restoring_session = true;
        let connected = self.connect_saved(saved);
        self.restoring_session = false;
        if connected.is_err() {
            return;
        }
        self.tabs.clear();
        self.query_counter = 0;
        for tab in &session.tabs {
            match tab {
                crate::settings::SessionTab::Query {
                    title,
                    sql,
                    row_limit,
                    scroll,
                } => self.push_restored_query_tab(title, sql, *row_limit, *scroll),
                crate::settings::SessionTab::Table {
                    schema,
                    name,
                    offset,
                    limit,
                    filter,
                    sort,
                    detail_open,
                    scroll,
                } => {
                    let table = TableId::with_schema(schema.clone(), name.clone());
                    self.open_table_tab_state(
                        table,
                        *offset,
                        *limit,
                        filter,
                        sort,
                        *detail_open,
                        *scroll,
                    );
                }
            }
        }
        if self.tabs.is_empty() {
            self.new_query_tab(true);
        } else {
            self.active = session.active.min(self.tabs.len() - 1);
        }
        self.last_session = Some(session.clone());
        self.push_toast(
            format!("Restored {}", session.connection_name),
            ToastKind::Info,
        );
    }

    fn push_restored_query_tab(
        &mut self,
        title: &str,
        sql: &str,
        row_limit: usize,
        scroll: f32,
    ) {
        self.query_counter += 1;
        let id = self.next_tab_id();
        let title = if title.trim().is_empty() {
            format!("Query {}", self.query_counter)
        } else {
            title.to_string()
        };
        self.tabs.push(Tab {
            id,
            title,
            kind: TabKind::Query(QueryTab {
                sql: sql.to_string(),
                outcome: None,
                running: false,
                mode: ResultMode::Result,
                selected: crate::widgets::GridSelection::default(),
                sort: Vec::new(),
                sorted: true,
                original_rows: None,
                row_limit,
                dirty: false,
                split: self.settings.editor_split,
                error_line: None,
                search: String::new(),
                cursor: 0,
                move_cursor: false,
                completion_index: 0,
                completion_word: String::new(),
                search_last: String::new(),
                widths: Vec::new(),
                scroll,
                restore_scroll: Some(scroll),
            }),
        });
    }

    /// Opens a table tab at a saved scroll/page/filter/sort position.
    #[allow(clippy::too_many_arguments)]
    fn open_table_tab_state(
        &mut self,
        table: TableId,
        offset: usize,
        limit: usize,
        filter: &db::TableFilter,
        sort: &[db::SortSpec],
        detail_open: bool,
        scroll: f32,
    ) {
        let Some(live) = &self.live else {
            return;
        };
        let sql_name = table.sql();
        let limit = if limit == 0 {
            db::DEFAULT_TABLE_PAGE_LIMIT
        } else {
            limit
        };
        let data = live
            .db
            .fetch_table(&sql_name, limit, offset, filter, sort);
        if data.error.is_some() && data.columns.is_empty() {
            return;
        }
        let structure = live.db.table_structure(&sql_name).unwrap_or_default();
        let indexes = live.db.table_indexes(&sql_name);
        let ddl = live.db.table_ddl(&sql_name);
        let id = self.next_tab_id();
        self.tabs.push(Tab {
            id,
            title: table.display(),
            kind: TabKind::Table(TableTab {
                table,
                mode: TableMode::Data,
                offset,
                limit,
                paging_open: false,
                limit_text: limit.to_string(),
                offset_text: offset.to_string(),
                paging_error: None,
                filter: filter.clone(),
                applied_filter: filter.clone(),
                filters_open: false,
                data,
                structure,
                indexes,
                ddl,
                selected: crate::widgets::GridSelection::default(),
                sort: sort.to_vec(),
                edit: None,
                detail_open,
                detail_search: String::new(),
                detail_match: DetailMatch::Fuzzy,
                detail_case: false,
                detail: None,
                widths: Vec::new(),
                scroll,
                restore_scroll: Some(scroll),
            }),
        });
        self.active = self.tabs.len().saturating_sub(1);
    }

    pub(crate) fn run_active(&mut self) {
        self.start_query(false);
    }

    /// Runs `EXPLAIN` (or `EXPLAIN QUERY PLAN` on SQLite) for the editor SQL.
    pub(crate) fn explain_active(&mut self) {
        self.start_query(true);
    }

    fn start_query(&mut self, explain: bool) {
        let idx = self.active;
        if self.live.is_none() {
            self.push_toast("Open a connection first.", ToastKind::Error);
            return;
        }
        if self.running_query.is_some() {
            self.push_toast(
                "A query is already running. Cancel it first.",
                ToastKind::Info,
            );
            return;
        }
        let Some(tab) = self.tabs.get(idx) else {
            return;
        };
        let TabKind::Query(query) = &tab.kind else {
            return;
        };
        if query.sql.trim().is_empty() {
            self.push_toast("There is no query to run.", ToastKind::Error);
            return;
        }
        let tab_id = tab.id;
        let sql = query.sql.clone();
        let max_rows = query.row_limit;
        let driver = self
            .live
            .as_ref()
            .map(|live| live.db.driver)
            .unwrap_or(Driver::Sqlite);

        // A statement with `:name` placeholders asks for values first.
        let names = crate::sqlfmt::parameters(&sql);
        let pending: Option<(u64, String, usize)> = match &self.params_dialog {
            Some(dialog) if dialog.tab_id == tab_id => {
                let values: Vec<(String, String)> = dialog
                    .names
                    .iter()
                    .cloned()
                    .zip(dialog.values.iter().cloned())
                    .collect();
                let filled = crate::sqlfmt::substitute_parameters(&sql, &values, driver);
                self.params_dialog = None;
                Some((tab_id, filled, max_rows))
            }
            _ => {
                if names.is_empty() {
                    Some((tab_id, sql, max_rows))
                } else {
                    self.params_dialog = Some(ParamsDialog {
                        tab_id,
                        names: names.clone(),
                        values: vec![String::new(); names.len()],
                    });
                    None
                }
            }
        };
        let Some((tab_id, sql, max_rows)) = pending else {
            return;
        };
        let sql = if explain {
            match driver {
                Driver::Sqlite => format!("EXPLAIN QUERY PLAN {sql}"),
                _ => format!("EXPLAIN {sql}"),
            }
        } else {
            sql
        };
        // Read-only blocks writes before any confirmation is shown.
        if self.is_read_only() && db::statement_writes(&sql) {
            self.push_toast(
                "Read-only mode is on: this statement would modify the database."
                    .to_string(),
                ToastKind::Error,
            );
            return;
        }
        // Safe mode asks before a write leaves the editor.
        if self.settings.safe_mode && !explain && db::statement_writes(&sql) {
            self.safe_query = Some(SafeQuery {
                tab_id,
                sql,
                max_rows,
            });
            return;
        }
        self.submit_query(tab_id, sql, max_rows, QueryKind::Sql);
    }

    /// Sends one statement to the worker and marks its tab as running.
    fn submit_query(&mut self, tab_id: u64, sql: String, max_rows: usize, kind: QueryKind) {
        let job_id = self.next_job_id();
        let read_only = self.is_read_only();
        let Some(worker) = &self.query_worker else {
            self.push_toast("The connection is closed.", ToastKind::Error);
            return;
        };
        let (control_tx, control_rx) = mpsc::channel();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        if worker
            .tx
            .send(QueryRequest {
                id: job_id,
                sql: sql.clone(),
                max_rows,
                read_only,
                control: control_tx,
                cancelled: cancel_flag.clone(),
            })
            .is_err()
        {
            self.push_toast("The query worker is not available.", ToastKind::Error);
            return;
        }
        if let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) {
            if let TabKind::Query(query) = &mut self.tabs[index].kind {
                query.running = true;
                query.selected.clear();
                query.error_line = None;
                query.dirty = false;
            }
        }
        if !kind.is_transaction() {
            self.record_history(sql);
        }
        self.running_query = Some(RunningQuery {
            id: job_id,
            kind,
            tab_id,
            started: Instant::now(),
            cancelled: false,
            cancel_flag,
            cancel: None,
            control_rx,
        });
    }

    /// Applies `sqlfmt::format_sql` to the active query tab.
    pub(crate) fn format_active_sql(&mut self) {
        let idx = self.active;
        let Some(tab) = self.tabs.get_mut(idx) else {
            return;
        };
        let TabKind::Query(query) = &mut tab.kind else {
            self.push_toast("Open a query tab to format SQL.", ToastKind::Info);
            return;
        };
        if query.sql.trim().is_empty() {
            return;
        }
        query.sql = crate::sqlfmt::format_sql(&query.sql);
        query.dirty = true;
        let cursor = query.sql.len();
        query.cursor = cursor;
        query.move_cursor = true;
        self.push_toast("Formatted SQL.", ToastKind::Info);
    }

    /// Inserts one of the editor templates at the caret.
    pub(crate) fn insert_sql_template(&mut self, name: &str) {
        let Some(template) = crate::sqlfmt::template(name) else {
            return;
        };
        let idx = self.active;
        let Some(tab) = self.tabs.get_mut(idx) else {
            return;
        };
        let TabKind::Query(query) = &mut tab.kind else {
            self.push_toast("Open a query tab to insert a template.", ToastKind::Info);
            return;
        };
        let at = query.cursor.min(query.sql.len());
        let before = query.sql[..at].to_string();
        let after = query.sql[at..].to_string();
        let mut insertion = template.replace("{}", "");
        if !before.is_empty() && !before.ends_with('\n') {
            insertion.insert_str(0, "\n\n");
        }
        if !after.is_empty() && !after.starts_with('\n') {
            insertion.push('\n');
        }
        query.sql = format!("{before}{insertion}{after}");
        query.cursor = before.len() + insertion.len();
        query.move_cursor = true;
        query.dirty = true;
    }

    /// Drops the running-query bookkeeping and clears the tab flag, if the tab
    /// still exists. Used when the workspace is replaced rather than when a
    /// query finishes normally.
    fn clear_running_state(&mut self) {
        let Some(job) = self.running_query.take() else {
            return;
        };
        if let Some(index) = self.tabs.iter().position(|tab| tab.id == job.tab_id) {
            if let TabKind::Query(query) = &mut self.tabs[index].kind {
                query.running = false;
            }
        }
    }

    /// Records a successful cell write in the undo journal. Called by the grid,
    /// the row detail panel and the value window so Cmd+Z covers all three.
    pub(crate) fn record_edit(
        &mut self,
        table: &TableId,
        key_column: &str,
        key_value: &str,
        column: &str,
        before: Option<String>,
        after: Option<String>,
    ) {
        if before == after {
            return;
        }
        self.edit_history.undo.push(EditRecord {
            table: table.clone(),
            key_column: key_column.to_string(),
            key_value: key_value.to_string(),
            column: column.to_string(),
            before,
            after,
        });
        if self.edit_history.undo.len() > 200 {
            self.edit_history.undo.remove(0);
        }
        self.edit_history.redo.clear();
    }

    /// Applies the inverse (or forward) value of one journal entry.
    fn apply_edit_record(&mut self, record: &EditRecord, value: Option<String>) {
        let table = record.table.sql();
        let Some(live) = &self.live else {
            return;
        };
        match live.db.update_cell(
            &table,
            &record.column,
            &record.key_column,
            &record.key_value,
            value.as_deref(),
        ) {
            Ok(_) => {
                if let Some(index) = self.tabs.iter().position(|tab| {
                    matches!(&tab.kind, TabKind::Table(tt) if tt.table == record.table)
                }) {
                    self.refresh_table_tab(index);
                }
            }
            Err(error) => self.push_toast(error, ToastKind::Error),
        }
    }

    pub(crate) fn undo_edit(&mut self) {
        let Some(record) = self.edit_history.undo.pop() else {
            self.push_toast("Nothing to undo.", ToastKind::Info);
            return;
        };
        self.apply_edit_record(&record, record.before.clone());
        self.edit_history.redo.push(record);
        self.push_toast("Undid the last edit.", ToastKind::Info);
    }

    pub(crate) fn redo_edit(&mut self) {
        let Some(record) = self.edit_history.redo.pop() else {
            self.push_toast("Nothing to redo.", ToastKind::Info);
            return;
        };
        self.apply_edit_record(&record, record.after.clone());
        self.edit_history.undo.push(record);
        self.push_toast("Redid the edit.", ToastKind::Info);
    }

    /// Re-applies the persistent settings to egui and writes them to disk.
    pub(crate) fn apply_settings(&mut self, ctx: &egui::Context) {
        self.settings.normalize();
        self.theme_dark = self.settings.dark;
        ctx.set_theme(if self.theme_dark {
            egui::ThemePreference::Dark
        } else {
            egui::ThemePreference::Light
        });
        self.palette = theme::apply(
            ctx,
            self.theme_dark,
            self.settings.data_font_size,
            self.settings.sql_font_size,
        );
        self.settings.save();
    }

    pub(crate) fn is_read_only(&self) -> bool {
        self.live
            .as_ref()
            .map(|live| live.db.is_read_only())
            .unwrap_or(false)
    }

    /// Turns read-only mode on/off and persists the choice on the connection.
    /// Turning it on closes any open editor transaction first, because the
    /// transaction could still contain pending writes.
    pub(crate) fn set_read_only(&mut self, value: bool) {
        if value && self.editor_tx.is_some() {
            self.rollback_transaction();
        }
        let saved = {
            let Some(live) = &mut self.live else {
                return;
            };
            live.db.set_read_only(value);
            live.saved.read_only = value;
            live.saved.clone()
        };
        self.store.update_connection(saved);
        self.push_toast(
            if value {
                "Read-only mode on: writes are blocked.".to_string()
            } else {
                "Read-only mode off: the database can be modified.".to_string()
            },
            ToastKind::Info,
        );
    }

    pub(crate) fn begin_transaction(&mut self) {
        self.submit_transaction(QueryKind::Begin);
    }

    pub(crate) fn commit_transaction(&mut self) {
        self.submit_transaction(QueryKind::Commit);
    }

    pub(crate) fn rollback_transaction(&mut self) {
        self.submit_transaction(QueryKind::Rollback);
    }

    fn submit_transaction(&mut self, kind: QueryKind) {
        if self.live.is_none() {
            self.push_toast("Open a connection first.", ToastKind::Error);
            return;
        }
        if self.running_query.is_some() {
            self.push_toast("Wait for the running query to finish first.", ToastKind::Info);
            return;
        }
        match kind {
            QueryKind::Begin if self.editor_tx.is_some() => {
                self.push_toast("A transaction is already open.", ToastKind::Info);
                return;
            }
            QueryKind::Commit | QueryKind::Rollback if self.editor_tx.is_none() => {
                self.push_toast("There is no open transaction.", ToastKind::Info);
                return;
            }
            _ => {}
        }
        let tab_id = match self.tabs.get(self.active).map(|tab| &tab.kind) {
            Some(TabKind::Query(_)) => self.tabs[self.active].id,
            _ => 0,
        };
        let sql = match kind {
            QueryKind::Begin => "BEGIN",
            QueryKind::Commit => "COMMIT",
            QueryKind::Rollback => "ROLLBACK",
            QueryKind::Sql => return,
        };
        self.submit_query(tab_id, sql.to_string(), 0, kind);
    }

    /// Applies the result of a BEGIN/COMMIT/ROLLBACK request.
    fn finish_transaction(
        &mut self,
        kind: QueryKind,
        tab_id: u64,
        outcome: db::QueryOutcome,
        cancelled: bool,
    ) {
        if let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) {
            if let TabKind::Query(query) = &mut self.tabs[index].kind {
                query.running = false;
            }
        }
        if cancelled {
            if kind != QueryKind::Begin {
                self.editor_tx = None;
            }
            self.push_toast("Transaction command cancelled.", ToastKind::Info);
            return;
        }
        if let Some(error) = outcome.error {
            // A failed COMMIT usually ends the transaction server-side.
            if kind != QueryKind::Begin {
                self.editor_tx = None;
            }
            self.push_toast(format!("Transaction error: {error}"), ToastKind::Error);
            return;
        }
        match kind {
            QueryKind::Begin => {
                self.editor_tx = Some(EditorTransaction {
                    started: Instant::now(),
                    statements: 0,
                });
                self.push_toast(
                    "Transaction started. Statements run in it until you commit or roll back."
                        .to_string(),
                    ToastKind::Info,
                );
            }
            QueryKind::Commit => {
                self.editor_tx = None;
                self.push_toast("Transaction committed.", ToastKind::Info);
            }
            QueryKind::Rollback => {
                self.editor_tx = None;
                self.push_toast("Transaction rolled back.", ToastKind::Info);
            }
            QueryKind::Sql => {}
        }
    }

    pub(crate) fn safe_query_window(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let Some(query) = self.safe_query.take() else {
            return;
        };
        let mut run = false;
        let mut cancel = false;
        egui::Window::new("Safe mode")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(Rounding::same(10.0))
                    .inner_margin(Margin::symmetric(20.0, 18.0)),
            )
            .show(ctx, |ui| {
                ui.set_width(460.0);
                ui.label(
                    RichText::new("This statement modifies the database")
                        .size(13.0)
                        .strong()
                        .color(p.text),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Safe mode asks before writes. You can turn it off in Settings.",
                    )
                    .size(10.6)
                    .color(p.muted),
                );
                ui.add_space(8.0);
                egui::Frame::none()
                    .fill(p.editor)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(Rounding::same(6.0))
                    .inner_margin(Margin::symmetric(10.0, 8.0))
                    .show(ui, |ui| {
                        ui.set_max_width(430.0);
                        let preview: String = query.sql.chars().take(600).collect();
                        ui.label(
                            RichText::new(preview)
                                .monospace()
                                .size(10.6)
                                .color(p.code),
                        );
                    });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let primary = egui::Button::new(
                        RichText::new("Run statement").size(12.0).color(Color32::WHITE),
                    )
                    .fill(p.accent)
                    .rounding(Rounding::same(6.0))
                    .min_size(Vec2::new(128.0, 28.0));
                    if ui.add(primary).clicked() {
                        run = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Cancel").size(12.0).color(p.muted),
                            )
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::NONE),
                        )
                        .clicked()
                    {
                        cancel = true;
                    }
                });
            });
        if run {
            self.submit_query(query.tab_id, query.sql, query.max_rows, QueryKind::Sql);
        } else if !cancel {
            self.safe_query = Some(query);
        }
    }

    pub(crate) fn toggle_theme(&mut self, ctx: &egui::Context) {
        self.settings.dark = !self.theme_dark;
        self.apply_settings(ctx);
    }

    fn zoom_fonts(&mut self, ctx: &egui::Context, delta: f32) {
        let next = (self.settings.data_font_size + delta).clamp(9.0, 20.0);
        self.settings.data_font_size = next;
        self.settings.sql_font_size = next;
        self.apply_settings(ctx);
    }

    fn reset_zoom(&mut self, ctx: &egui::Context) {
        self.settings.data_font_size = theme::DATA_FONT_SIZE;
        self.settings.sql_font_size = theme::SQL_FONT_SIZE;
        self.apply_settings(ctx);
    }

    /// One entry point for the native menu, the command palette and the
    /// keyboard shortcuts.
    pub(crate) fn dispatch_command(&mut self, command: crate::menu::CommandId, ctx: &egui::Context) {
        use crate::menu::CommandId;
        if let Some((last, at)) = self.last_command {
            if last == command && at.elapsed() < Duration::from_millis(200) {
                return;
            }
        }
        self.last_command = Some((command, Instant::now()));
        if command.needs_connection() && self.live.is_none() {
            self.push_toast("Open a connection first.", ToastKind::Info);
            return;
        }
        if command.needs_query_tab()
            && !matches!(
                self.tabs.get(self.active).map(|tab| &tab.kind),
                Some(TabKind::Query(_))
            )
            && !matches!(command, CommandId::Find)
        {
            self.push_toast("This action needs a query tab.", ToastKind::Info);
            return;
        }
        match command {
            CommandId::NewConnection => self.open_new_connection_dialog(),
            CommandId::NewQueryTab => self.new_query_tab(true),
            CommandId::CloseTab => self.close_tab(self.active),
            CommandId::DuplicateTab => self.duplicate_active_tab(),
            CommandId::Save => self.save_active(),
            CommandId::ExportSql => {
                self.open_db_tools();
                if let Some(tools) = &mut self.db_tools {
                    tools.tab = ToolsTab::Export;
                }
            }
            CommandId::ImportSql => {
                self.open_db_tools();
                if let Some(tools) = &mut self.db_tools {
                    tools.tab = ToolsTab::Import;
                }
            }
            CommandId::ImportData => {
                if let Some(path) = crate::import::pick_data_file() {
                    self.open_import_dialog(path);
                }
            }
            CommandId::Settings => self.settings_open = true,
            CommandId::Undo => self.undo_edit(),
            CommandId::Redo => self.redo_edit(),
            CommandId::Find => self.focus_search = true,
            CommandId::CommandPalette => {
                self.palette_open = true;
                self.palette_search.clear();
                self.palette_index = 0;
            }
            CommandId::RunQuery => self.run_active(),
            CommandId::Explain => self.explain_active(),
            CommandId::FormatSql => self.format_active_sql(),
            CommandId::BeginTransaction => self.begin_transaction(),
            CommandId::CommitTransaction => self.commit_transaction(),
            CommandId::RollbackTransaction => self.rollback_transaction(),
            CommandId::ToggleReadOnly => {
                let next = !self.is_read_only();
                self.set_read_only(next);
            }
            CommandId::TemplateSelect => self.insert_sql_template("SELECT"),
            CommandId::TemplateJoin => self.insert_sql_template("JOIN"),
            CommandId::TemplateInsert => self.insert_sql_template("INSERT"),
            CommandId::TemplateUpdate => self.insert_sql_template("UPDATE"),
            CommandId::TemplateDelete => self.insert_sql_template("DELETE"),
            CommandId::TemplateCreateTable => self.insert_sql_template("CREATE TABLE"),
            CommandId::ToggleTheme => self.toggle_theme(ctx),
            CommandId::ZoomIn => self.zoom_fonts(ctx, 1.0),
            CommandId::ZoomOut => self.zoom_fonts(ctx, -1.0),
            CommandId::ZoomReset => self.reset_zoom(ctx),
            CommandId::ToggleRowDetail => self.toggle_row_detail(),
            CommandId::About => self.about_open = true,
        }
    }

    fn duplicate_active_tab(&mut self) {
        let idx = self.active;
        let Some(tab) = self.tabs.get(idx) else {
            return;
        };
        match &tab.kind {
            TabKind::Query(query) => {
                let sql = query.sql.clone();
                let limit = query.row_limit;
                let title = format!("{} copy", tab.title);
                self.query_counter += 1;
                let id = self.next_tab_id();
                self.tabs.push(Tab {
                    id,
                    title,
                    kind: TabKind::Query(QueryTab {
                        sql,
                        outcome: None,
                        running: false,
                        mode: ResultMode::Result,
                        selected: crate::widgets::GridSelection::default(),
                        sort: Vec::new(),
                        sorted: true,
                        original_rows: None,
                        row_limit: limit,
                        dirty: true,
                        split: self.settings.editor_split,
                        error_line: None,
                        search: String::new(),
                        cursor: 0,
                        move_cursor: false,
                        completion_index: 0,
                        completion_word: String::new(),
                        search_last: String::new(),
                        widths: Vec::new(),
                scroll: 0.0,
                restore_scroll: None,
                    }),
                });
                self.active = self.tabs.len() - 1;
            }
            TabKind::Table(table) => {
                let table = table.table.clone();
                self.open_table_tab(table);
            }
        }
    }

    /// Cmd+S: commits the open cell editor and marks the tab as saved.
    pub(crate) fn save_active(&mut self) {
        let idx = self.active;
        self.flush_row_detail_edits(idx);
        // The pending cell edit is extracted first so the write can borrow the
        // connection without holding the tab borrow.
        let pending = self.tabs.get_mut(idx).and_then(|tab| match &mut tab.kind {
            TabKind::Query(query) => {
                query.dirty = false;
                None
            }
            TabKind::Table(table) => {
                let edit = table.edit.take()?;
                let key_column = table.data.key_column.clone()?;
                let key_value = table.data.rowids.get(edit.row).cloned().flatten()?;
                let column = table.data.columns.get(edit.col).cloned()?;
                let value = crate::views::edited_cell_value(&edit.original, &edit.text);
                Some((
                    table.table.clone(),
                    column,
                    key_column,
                    key_value,
                    edit.original,
                    value,
                ))
            }
        });
        let committed = if let Some((table, column, key_column, key_value, before, value)) = pending
        {
            let sql_table = table.sql();
            let result = match &self.live {
                Some(live) => live.db.update_cell(
                    &sql_table,
                    &column,
                    &key_column,
                    &key_value,
                    value.as_deref(),
                ),
                None => Err("The connection is closed.".to_string()),
            };
            match result {
                Ok(_) => {
                    self.record_edit(&table, &key_column, &key_value, &column, before, value);
                    true
                }
                Err(error) => {
                    self.push_toast(error, ToastKind::Error);
                    false
                }
            }
        } else {
            false
        };
        self.autosave_session(true);
        if committed {
            self.refresh_table_tab(idx);
            self.push_toast("Changes saved.", ToastKind::Info);
        } else {
            self.push_toast("Saved.", ToastKind::Info);
        }
    }

    pub(crate) fn has_unsaved_sql(&self) -> bool {
        self.tabs.iter().any(|tab| match &tab.kind {
            TabKind::Query(query) => query.dirty,
            TabKind::Table(_) => false,
        })
    }

    /// Asks the background query to stop. The actual driver call happens on a
    /// short-lived thread because PostgreSQL/MySQL cancellation needs a
    /// separate connection.
    pub(crate) fn cancel_query(&mut self) {
        let Some(job) = &mut self.running_query else {
            return;
        };
        if job.cancelled {
            return;
        }
        job.cancelled = true;
        job.cancel_flag.store(true, Ordering::SeqCst);
        if let Some(handle) = job.cancel.take() {
            spawn_query_cancel(handle);
        }
        self.push_toast("Cancelling query…", ToastKind::Info);
    }

    /// Applies a finished background query and clears the running state.
    fn poll_query_job(&mut self, ctx: &egui::Context) {
        let mut finished: Option<(u64, db::QueryOutcome, bool, u128, QueryKind)> = None;
        let mut disconnected_tab: Option<u64> = None;

        if let Some(job) = &mut self.running_query {
            while let Ok(handle) = job.control_rx.try_recv() {
                if job.cancelled {
                    spawn_query_cancel(handle);
                } else {
                    job.cancel = Some(handle);
                }
            }
            let tab_id = job.tab_id;
            let job_id = job.id;
            let kind = job.kind;
            let cancelled = job.cancelled;
            let elapsed_ms = job.started.elapsed().as_millis();
            match self.query_worker.as_ref() {
                Some(worker) => loop {
                    match worker.result_rx.try_recv() {
                        Ok((id, outcome)) if id == job_id => {
                            finished = Some((tab_id, outcome, cancelled, elapsed_ms, kind));
                            break;
                        }
                        // A result left over from an abandoned request.
                        Ok(_) => continue,
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => {
                            disconnected_tab = Some(tab_id);
                            break;
                        }
                    }
                },
                None => disconnected_tab = Some(tab_id),
            }
        }

        if let Some((tab_id, mut outcome, cancelled, elapsed_ms, query_kind)) = finished {
            if query_kind.is_transaction() {
                self.running_query = None;
                self.finish_transaction(query_kind, tab_id, outcome, cancelled);
                ctx.request_repaint();
                return;
            }
            if cancelled {
                outcome = db::QueryOutcome {
                    error: Some("Query cancelled.".to_string()),
                    elapsed_ms,
                    ..db::QueryOutcome::default()
                };
            }
            let (message, kind) = if cancelled {
                ("Query cancelled.".to_string(), ToastKind::Info)
            } else if let Some(error) = &outcome.error {
                (format!("Query error: {error}"), ToastKind::Error)
            } else {
                (outcome.ok_message(), ToastKind::Info)
            };
            self.running_query = None;
            let succeeded = outcome.error.is_none() && !cancelled;
            if succeeded {
                if let Some(tx) = &mut self.editor_tx {
                    tx.statements += 1;
                }
            }
            let error_line = outcome
                .error
                .as_deref()
                .and_then(crate::sqlfmt::error_line);
            if let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) {
                if let TabKind::Query(query) = &mut self.tabs[index].kind {
                    query.running = false;
                    query.outcome = Some(outcome);
                    query.selected.clear();
                    query.sort.clear();
                    query.sorted = true;
                    query.original_rows = None;
                    query.error_line = error_line;
                }
                self.push_toast(message, kind);
            } else if cancelled {
                self.push_toast(message, kind);
            }
            self.refresh_schema();
            ctx.request_repaint();
        } else if let Some(tab_id) = disconnected_tab {
            self.running_query = None;
            if let Some(index) = self.tabs.iter().position(|tab| tab.id == tab_id) {
                if let TabKind::Query(query) = &mut self.tabs[index].kind {
                    query.running = false;
                    query.selected.clear();
                    query.sort.clear();
                    query.sorted = true;
                    query.original_rows = None;
                    query.outcome = Some(db::QueryOutcome {
                        error: Some("The query worker stopped unexpectedly.".to_string()),
                        ..db::QueryOutcome::default()
                    });
                }
            }
            self.push_toast("The query worker stopped unexpectedly.", ToastKind::Error);
            ctx.request_repaint();
        }
    }

    fn record_history(&mut self, sql: String) {
        let sql = sql.trim().to_string();
        if sql.is_empty() || self.query_history.last() == Some(&sql) {
            return;
        }
        self.query_history.push(sql);
        db::save_query_history(&self.query_history);
    }

    pub(crate) fn refresh_table_tab(&mut self, idx: usize) {
        let Some(live) = &self.live else {
            return;
        };
        let Some(tab) = self.tabs.get_mut(idx) else {
            return;
        };
        if let TabKind::Table(tt) = &mut tab.kind {
            let sql_name = tt.table.sql();
            tt.data = live
                .db
                .fetch_table(
                    &sql_name,
                    tt.limit,
                    tt.offset,
                    &tt.applied_filter,
                    &tt.sort,
                );
            tt.structure = live.db.table_structure(&sql_name).unwrap_or_default();
            tt.indexes = live.db.table_indexes(&sql_name);
            tt.ddl = live.db.table_ddl(&sql_name);
            tt.selected.clear();
            tt.edit = None;
            tt.detail = None;
        }
    }

    fn page_active(&mut self, forward: bool) {
        let idx = self.active;
        let mut refresh = false;
        if let Some(tab) = self.tabs.get_mut(idx) {
            if let TabKind::Table(tt) = &mut tab.kind {
                // `sub_10071b144` / `sub_10071b370`: the step is
                // `max(limit, 300)`, so "No limit" still pages by 300 rows.
                let step = db::page_step(tt.limit);
                if forward {
                    if ((tt.offset + step) as i64) < tt.data.total {
                        tt.offset += step;
                        refresh = true;
                    }
                } else if tt.offset > 0 {
                    tt.offset = tt.offset.saturating_sub(step);
                    refresh = true;
                }
            }
        }
        if refresh {
            self.refresh_table_tab(idx);
        }
    }

    /// `WorkspaceWindow.nib` menu item "Toggle Row Detail (Space)" calls
    /// `toggleRightSidebar:` on the workspace. In a table's Data view the
    /// space bar toggles the vertical row editor.
    pub(crate) fn toggle_row_detail(&mut self) {
        if let Some(tab) = self.tabs.get_mut(self.active) {
            if let TabKind::Table(tt) = &mut tab.kind {
                if tt.mode == TableMode::Data {
                    tt.detail_open = !tt.detail_open;
                }
            }
        }
    }

    pub(crate) fn push_toast(&mut self, text: impl Into<String>, kind: ToastKind) {
        if self.screenshot.is_some() {
            return;
        }
        self.toast = Some(Toast {
            text: text.into(),
            kind,
            until: Instant::now() + Duration::from_secs(4),
        });
    }

    // ------------------------------------------------------------------
    // UI: chrome
    // ------------------------------------------------------------------

    fn topbar(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let mut do_run = false;
        let mut do_cancel = false;
        let mut do_new_query = false;
        let mut do_disconnect = false;
        let mut do_toggle_theme = false;
        let mut do_about = false;
        let mut do_open_tools = false;
        let mut do_toggle_detail = false;
        let mut do_toggle_read_only = false;
        let read_only = self.is_read_only();

        egui::TopBottomPanel::top("titlebar")
            .exact_height(46.0)
            .frame(
                egui::Frame::none()
                    .fill(p.topbar)
                    .inner_margin(Margin::symmetric(12.0, 0.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    crate::widgets::draw_logo(ui, 24.0, &p);
                    ui.add_space(1.0);
                    ui.label(RichText::new("Dataloom").size(14.5).strong().color(p.text));
                    ui.add_space(4.0);
                    let driver = self
                        .live
                        .as_ref()
                        .map(|live| live.db.driver)
                        .unwrap_or(Driver::Sqlite);
                    crate::widgets::chip(
                        ui,
                        &p,
                        driver.label(),
                        crate::widgets::driver_color(&p, driver),
                    );

                    if let Some(live) = &self.live {
                        ui.add_space(10.0);
                        ui.label(RichText::new("·").color(p.faint));
                        ui.add_space(2.0);
                        crate::widgets::dot(ui, p.green, 6.0);
                        ui.label(
                            RichText::new(crate::widgets::truncate(&live.saved.name, 26))
                                .size(12.8)
                                .color(p.text),
                        );
                        ui.label(
                            RichText::new(crate::widgets::truncate(&live.saved.target(), 52))
                                .size(11.5)
                                .color(p.faint),
                        );
                    } else {
                        ui.add_space(10.0);
                        crate::widgets::dot(ui, p.faint, 6.0);
                        ui.label(RichText::new("No connection").size(12.5).color(p.muted));
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if crate::widgets::icon_toggle(ui, &p, self.theme_dark).clicked() {
                            do_toggle_theme = true;
                        }
                        if crate::widgets::info_button(ui, &p).clicked() {
                            do_about = true;
                        }
                        if self.live.is_some() {
                            let label = if read_only { "Read-only" } else { "Read-write" };
                            let chip = egui::Button::new(
                                RichText::new(label).size(10.4).color(if read_only {
                                    p.amber
                                } else {
                                    p.faint
                                }),
                            )
                            .fill(if read_only {
                                p.accent_soft
                            } else {
                                Color32::TRANSPARENT
                            })
                            .stroke(Stroke::new(1.0_f32, p.border))
                            .rounding(Rounding::same(5.0));
                            if ui
                                .add(chip)
                                .on_hover_text("Toggle read-only mode (Shift+Cmd+L)")
                                .clicked()
                            {
                                do_toggle_read_only = true;
                            }
                        }
                        if self.live.is_some()
                            && ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Disconnect").size(12.0).color(p.muted),
                                    )
                                    .fill(Color32::TRANSPARENT)
                                    .stroke(Stroke::NONE),
                                )
                                .clicked()
                        {
                            do_disconnect = true;
                        }
                        if self.live.is_some()
                            && crate::widgets::icon_button(ui, &p, "SQL tools")
                                .on_hover_text("Export, import and back up this database")
                                .clicked()
                        {
                            do_open_tools = true;
                        }
                        if self.live.is_some() {
                            if crate::widgets::plus_button(ui, &p, "New query").clicked() {
                                do_new_query = true;
                            }
                            if self.running_query.is_some() {
                                if crate::widgets::icon_button(ui, &p, "Cancel").clicked() {
                                    do_cancel = true;
                                }
                            } else {
                                let run_enabled = matches!(
                                    self.tabs.get(self.active).map(|t| &t.kind),
                                    Some(TabKind::Query(q)) if !q.running
                                );
                                if crate::widgets::run_button(ui, &p, run_enabled).clicked()
                                    && run_enabled
                                {
                                    do_run = true;
                                }
                            }
                            // The row detail toggle lives in the main toolbar
                            // like the original's Panels control. It stays
                            // visible while the panel is closed, labelled, so
                            // the panel can always be brought back.
                            let (detail_enabled, detail_open) = match self
                                .tabs
                                .get(self.active)
                                .map(|t| &t.kind)
                            {
                                Some(TabKind::Table(tt))
                                    if tt.mode == TableMode::Data =>
                                {
                                    (true, tt.detail_open)
                                }
                                _ => (false, false),
                            };
                            ui.add_enabled_ui(detail_enabled, |ui| {
                                if crate::widgets::row_detail_toggle(
                                    ui, &p, detail_open,
                                )
                                .on_hover_text(
                                    "Show or hide the vertical row editor (Space)",
                                )
                                .clicked()
                                {
                                    do_toggle_detail = true;
                                }
                            });
                        }
                    });
                });
            });

        if do_toggle_theme {
            self.toggle_theme(ctx);
        }
        if do_toggle_read_only {
            let next = !self.is_read_only();
            self.set_read_only(next);
        }
        if do_new_query {
            self.new_query_tab(true);
        }
        if do_about {
            self.about_open = true;
        }
        if do_disconnect {
            self.disconnect();
        }
        if do_run {
            self.run_active();
        }
        if do_cancel {
            self.cancel_query();
        }
        if do_open_tools {
            self.open_db_tools();
        }
        if do_toggle_detail {
            self.toggle_row_detail();
        }
    }

    fn tabbar(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let mut select = None;
        let mut close = None;
        let mut new_query = false;

        egui::TopBottomPanel::top("tabbar")
            .exact_height(38.0)
            .frame(
                egui::Frame::none()
                    .fill(p.tabbar)
                    .inner_margin(Margin::symmetric(8.0, 4.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    for (i, tab) in self.tabs.iter().enumerate() {
                        let selected = i == self.active;
                        let width = if matches!(tab.kind, TabKind::Table(_)) {
                            178.0
                        } else {
                            158.0
                        };
                        let (rect, resp) =
                            ui.allocate_exact_size(Vec2::new(width, 28.0), Sense::click());
                        let bg = if selected {
                            p.panel
                        } else if resp.hovered() {
                            p.row_hover
                        } else {
                            Color32::TRANSPARENT
                        };
                        let painter = ui.painter().clone();
                        painter.rect_filled(
                            rect,
                            Rounding {
                                nw: 6.0,
                                ne: 6.0,
                                sw: 0.0,
                                se: 0.0,
                            },
                            bg,
                        );
                        if selected {
                            painter.rect_filled(
                                Rect::from_min_size(rect.left_top(), Vec2::new(width, 2.0)),
                                Rounding::same(1.0),
                                p.accent,
                            );
                        }

                        let icon_center = egui::pos2(rect.left() + 17.0, rect.center().y);
                        match &tab.kind {
                            TabKind::Query(q) => {
                                painter.text(
                                    icon_center,
                                    Align2::CENTER_CENTER,
                                    "</>",
                                    FontId::monospace(9.0),
                                    if selected || q.running { p.accent } else { p.faint },
                                );
                            }
                            TabKind::Table(_) => {
                                crate::widgets::draw_table_icon(
                                    &painter,
                                    Rect::from_center_size(icon_center, Vec2::splat(12.0)),
                                    if selected { p.accent } else { p.faint },
                                );
                            }
                        }

                        painter.text(
                            egui::pos2(rect.left() + 29.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            crate::widgets::truncate(&tab.title, 20),
                            FontId::proportional(12.3),
                            if selected { p.text } else { p.muted },
                        );

                        let x_rect = Rect::from_center_size(
                            egui::pos2(rect.right() - 13.0, rect.center().y),
                            Vec2::splat(15.0),
                        );
                        if resp.hovered() || selected {
                            crate::widgets::draw_cross(&painter, x_rect.shrink(4.0), p.faint);
                        }

                        if resp.clicked() {
                            if let Some(pos) = resp.interact_pointer_pos() {
                                if x_rect.contains(pos) {
                                    close = Some(i);
                                } else {
                                    select = Some(i);
                                }
                            }
                        }
                        let _ = resp.on_hover_text(&tab.title);
                    }

                    let (plus_rect, plus_resp) =
                        ui.allocate_exact_size(Vec2::new(28.0, 28.0), Sense::click());
                    if plus_resp.hovered() {
                        ui.painter()
                            .rect_filled(plus_rect, Rounding::same(6.0), p.row_hover);
                    }
                    crate::widgets::draw_plus(
                        ui.painter(),
                        plus_rect.center(),
                        7.0,
                        if plus_resp.hovered() { p.text } else { p.muted },
                    );
                    if plus_resp.clicked() {
                        new_query = true;
                    }

                    if self.tabs.is_empty() {
                        ui.add_space(8.0);
                        ui.label(RichText::new("No tabs open").size(11.5).color(p.faint));
                    }
                });
            });

        if let Some(i) = select {
            self.active = i;
        }
        if let Some(i) = close {
            self.close_tab(i);
        }
        if new_query {
            self.new_query_tab(true);
        }
    }

    fn statusbar(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let mut left = "No connection".to_string();
        let mut detail = String::new();
        if let Some(live) = &self.live {
            left = format!("Connected  ·  {}", live.saved.name);
        }
        if let Some(tab) = self.tabs.get(self.active) {
            match &tab.kind {
                TabKind::Query(q) => {
                    if q.running {
                        let job = self
                            .running_query
                            .as_ref()
                            .filter(|job| job.tab_id == tab.id);
                        let label = if job.map(|job| job.cancelled).unwrap_or(false) {
                            "Cancelling…"
                        } else {
                            "Running…"
                        };
                        detail = match job {
                            Some(job) => format!(
                                "{label}  ·  {:.1}s",
                                job.started.elapsed().as_secs_f32()
                            ),
                            None => label.to_string(),
                        };
                    } else if let Some(out) = &q.outcome {
                        detail = if let Some(_err) = &out.error {
                            format!("Error  ·  {}", crate::widgets::fmt_ms(out.elapsed_ms))
                        } else {
                            format!(
                                "{} row{}  ·  {}",
                                out.rows.len(),
                                if out.rows.len() == 1 { "" } else { "s" },
                                crate::widgets::fmt_ms(out.elapsed_ms)
                            )
                        };
                    } else {
                        detail = "Ready".to_string();
                    }
                }
                TabKind::Table(t) => {
                    // The page number follows the same `max(limit, 300)` step
                    // used by `sub_10081f074` for the Limit/Offset popover.
                    let page = t.offset / db::page_step(t.limit) + 1;
                    // Status wording from `sub_10000e300` (0x10000e300):
                    // "Rows (%@ of %@ loaded)" while the total is known, and
                    // "Rows (%@ so far)" when it is not.
                    let loaded = t.data.rows.len();
                    let counts = if t.data.total > 0 {
                        format!("Rows ({} of {} loaded)", loaded, t.data.total)
                    } else {
                        format!("Rows ({} so far)", loaded)
                    };
                    detail = format!("{}  ·  page {}", counts, page);
                }
            }
        }

        egui::TopBottomPanel::bottom("statusbar")
            .exact_height(27.0)
            .frame(
                egui::Frame::none()
                    .fill(p.topbar)
                    .inner_margin(Margin::symmetric(12.0, 0.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    crate::widgets::dot(
                        ui,
                        if self.live.is_some() {
                            p.green
                        } else {
                            p.faint
                        },
                        6.0,
                    );
                    ui.label(RichText::new(left).size(11.4).color(p.muted));
                    ui.add_space(10.0);
                    ui.label(RichText::new("UTF-8").size(11.2).color(p.faint));
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(format!("Tab {}", theme::TAB_WIDTH))
                            .size(11.2)
                            .color(p.faint),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new(
                            self.live
                                .as_ref()
                                .map(|live| live.db.driver.label())
                                .unwrap_or("SQLite"),
                        )
                        .size(11.2)
                        .color(p.faint),
                    );
                    if self.is_read_only() {
                        ui.add_space(8.0);
                        crate::widgets::chip(ui, &p, "read-only", p.amber);
                    }
                    if let Some(tx) = &self.editor_tx {
                        ui.add_space(8.0);
                        crate::widgets::chip(
                            ui,
                            &p,
                            &format!("transaction {}", crate::widgets::fmt_duration(tx.started.elapsed())),
                            p.purple,
                        );
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(detail).size(11.4).color(p.muted));
                    });
                });
            });
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let mut open_table = None;
        let mut open_connection = false;
        let mut sample = false;
        let mut menu_action: Option<TableMenuAction> = None;

        egui::SidePanel::left("sidebar")
            .exact_width(SIDEBAR_W)
            .resizable(false)
            .frame(egui::Frame::none().fill(p.sidebar).inner_margin(Margin {
                left: 10.0,
                right: 10.0,
                top: 12.0,
                bottom: 10.0,
            }))
            .show(ctx, |ui| {
                if let Some(live) = &self.live {
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
                        ui.painter()
                            .rect_filled(rect, Rounding::same(8.0), p.accent_soft);
                        ui.painter().text(
                            rect.center(),
                            Align2::CENTER_CENTER,
                            crate::widgets::initials(&live.saved.name),
                            FontId::proportional(12.0),
                            p.accent,
                        );
                        ui.add_space(2.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.label(
                                RichText::new(crate::widgets::truncate(&live.saved.name, 24))
                                    .size(13.0)
                                    .strong()
                                    .color(p.text),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "{}  ·  {}",
                                    live.db.driver.label(),
                                    if live.tunnel.is_some() {
                                        "SSH tunnel"
                                    } else {
                                        "connected"
                                    }
                                ))
                                    .size(10.5)
                                    .color(p.muted),
                            );
                        });
                    });
                    ui.add_space(10.0);
                } else {
                    ui.label(
                        RichText::new("DATABASE CONNECTIONS")
                            .size(10.5)
                            .color(p.faint),
                    );
                    ui.add_space(8.0);
                }

                if self.live.is_some() {
                    let search_frame = egui::Frame::none()
                        .fill(if p.dark {
                            Color32::from_rgb(30, 32, 38)
                        } else {
                            Color32::from_rgb(233, 236, 241)
                        })
                        .stroke(Stroke::new(1.0_f32, p.border))
                        .rounding(Rounding::same(6.0))
                        .inner_margin(Margin::symmetric(8.0, 5.0));
                    search_frame.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let icon_rect =
                                Rect::from_min_size(ui.cursor().min, Vec2::new(18.0, 16.0));
                            crate::widgets::draw_search_icon(ui.painter(), icon_rect, p.faint);
                            ui.add_space(13.0);
                            ui.add(
                                TextEdit::singleline(&mut self.sidebar.search)
                                    .frame(false)
                                    .hint_text("Search tables…")
                                    .desired_width(f32::INFINITY),
                            );
                        });
                    });
                    ui.add_space(10.0);
                } else {
                    ui.add_space(4.0);
                }

                let footer_height = if self.live.is_some() { 62.0 } else { 84.0 };
                let available = (ui.available_height() - footer_height).max(90.0);
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .max_height(available)
                    .show(ui, |ui| {
                        let q = self.sidebar.search.trim().to_lowercase();
                        if let Some(live) = &self.live {
                            let active_table =
                                self.tabs.get(self.active).and_then(|t| match &t.kind {
                                    TabKind::Table(tt) => Some(tt.table.clone()),
                                    _ => None,
                                });
                            let count = live
                                .tables
                                .iter()
                                .filter(|t| {
                                    q.is_empty() || t.id().display().to_lowercase().contains(&q)
                                })
                                .count();
                            let _ = crate::widgets::section_header(
                                ui,
                                &p,
                                "TABLES",
                                count,
                                &mut self.sidebar.tables_open,
                            );
                            if self.sidebar.tables_open {
                                for table in &live.tables {
                                    if !q.is_empty()
                                        && !table.id().display().to_lowercase().contains(&q)
                                    {
                                        continue;
                                    }
                                    let id = table.id();
                                    let selected = active_table.as_ref() == Some(&id);
                                    let resp =
                                        crate::widgets::table_row(ui, &p, table, selected);
                                    if resp.clicked() {
                                        open_table = Some(id.clone());
                                    }
                                    // TablePlus opens a table menu on right-click;
                                    // this build keeps only the five supported
                                    // operations (export/import/clone/truncate/
                                    // delete) plus the export format submenu.
                                    resp.context_menu(|ui| {
                                        table_menu_contents(
                                            ui,
                                            &id,
                                            self.live
                                                .as_ref()
                                                .map(|live| live.db.is_read_only())
                                                .unwrap_or(false),
                                            &mut menu_action,
                                        );
                                    });
                                }
                                if count == 0 {
                                    ui.label(
                                        RichText::new("No matching tables")
                                            .size(11.2)
                                            .color(p.faint),
                                    );
                                }
                            }
                            ui.add_space(8.0);
                            let vcount = live
                                .views
                                .iter()
                                .filter(|t| {
                                    q.is_empty() || t.id().display().to_lowercase().contains(&q)
                                })
                                .count();
                            let _ = crate::widgets::section_header(
                                ui,
                                &p,
                                "VIEWS",
                                vcount,
                                &mut self.sidebar.views_open,
                            );
                            if self.sidebar.views_open {
                                for view in &live.views {
                                    if !q.is_empty()
                                        && !view.id().display().to_lowercase().contains(&q)
                                    {
                                        continue;
                                    }
                                    let id = view.id();
                                    let selected = active_table.as_ref() == Some(&id);
                                    if crate::widgets::table_row(ui, &p, view, selected).clicked() {
                                        open_table = Some(id.clone());
                                    }
                                }
                                if vcount == 0 {
                                    ui.label(
                                        RichText::new("No views in this database")
                                            .size(11.2)
                                            .color(p.faint),
                                    );
                                }
                            }
                        } else {
                            ui.label(
                                RichText::new("Open a connection to browse tables.")
                                    .size(11.5)
                                    .color(p.faint),
                            );
                        }
                    });

                ui.add_space(6.0);
                let new_btn =
                    egui::Button::new(RichText::new("+   New connection").size(12.0).color(p.text))
                        .fill(p.card)
                        .stroke(Stroke::new(1.0_f32, p.border))
                        .rounding(Rounding::same(6.0))
                        .min_size(Vec2::new(ui.available_width(), 28.0));
                if ui.add(new_btn).clicked() {
                    open_connection = true;
                }
                ui.add_space(4.0);
                if self.live.is_none() {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Use sample database")
                                    .size(11.2)
                                    .color(p.muted),
                            )
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::NONE),
                        )
                        .clicked()
                    {
                        sample = true;
                    }
                } else {
                    ui.label(
                        RichText::new("Cmd+Enter runs the current query")
                            .size(10.3)
                            .color(p.faint),
                    );
                }
            });

        if let Some(table) = open_table {
            self.open_table_tab(table);
        }
        if let Some(action) = menu_action {
            self.handle_table_menu(action);
        }
        if open_connection {
            self.open_new_connection_dialog();
        }
        if sample {
            self.open_sample();
        }
    }

    fn central(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(self.palette.bg))
            .show(ctx, |ui| {
                if self.live.is_none() {
                    self.welcome_ui(ui);
                    return;
                }
                let kind = self
                    .tabs
                    .get(self.active)
                    .map(|t| matches!(t.kind, TabKind::Table(_)));
                match kind {
                    Some(true) => self.table_tab_ui(ui),
                    Some(false) => self.query_tab_ui(ui),
                    None => self.no_tabs_ui(ui),
                }
            });
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let mut open = self.about_open;
        let mut close = false;
        egui::Window::new("About Dataloom")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(Rounding::same(12.0))
                    .inner_margin(Margin::symmetric(28.0, 24.0)),
            )
            .show(ctx, |ui| {
                ui.set_width(300.0);
                ui.vertical_centered(|ui| {
                    crate::widgets::draw_logo(ui, 68.0, &p);
                    ui.add_space(10.0);
                    ui.label(RichText::new("Dataloom").size(20.0).strong().color(p.text));
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                            .size(11.5)
                            .color(p.muted),
                    );
                    ui.add_space(10.0);
                    ui.label(
                        RichText::new(
                            "A native SQLite, MySQL and PostgreSQL workspace rebuilt in Rust + egui.",
                        )
                        .size(11.4)
                        .color(p.muted),
                    );
                    ui.add_space(16.0);
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Close").size(12.0).color(p.text))
                                .fill(p.card)
                                .stroke(Stroke::new(1.0_f32, p.border))
                                .rounding(Rounding::same(6.0))
                                .min_size(Vec2::new(84.0, 28.0)),
                        )
                        .clicked()
                    {
                        close = true;
                    }
                });
            });
        self.about_open = open && !close;
    }
}

/// Runs a driver cancellation off the UI thread. PostgreSQL and MySQL
/// cancellation open a short-lived network connection, so they must not block
/// the event loop; SQLite's interrupt is immediate but harmless here.
fn spawn_query_cancel(handle: db::QueryCancel) {
    let _ = thread::Builder::new()
        .name("dataloom-query-cancel".to_string())
        .spawn(move || {
            if let Err(error) = handle.cancel() {
                eprintln!("Could not cancel the query: {error}");
            }
        });
}

/// Default column mapping: match by name first, then fill unmatched targets by
/// position. Every target is used at most once.
fn default_import_mapping(
    targets: &[crate::import::TargetColumn],
    source_columns: &[String],
) -> Vec<Option<usize>> {
    let mut used = vec![false; targets.len()];
    let mut mapping = vec![None; source_columns.len()];
    for (index, column) in source_columns.iter().enumerate() {
        if let Some(target) = targets
            .iter()
            .position(|target| target.name.eq_ignore_ascii_case(column))
        {
            if !used[target] {
                mapping[index] = Some(target);
                used[target] = true;
            }
        }
    }
    for index in 0..source_columns.len() {
        if mapping[index].is_none() && index < targets.len() && !used[index] {
            mapping[index] = Some(index);
            used[index] = true;
        }
    }
    mapping
}

/// A connection name that does not collide with `existing`: keeps the desired
/// name when free, otherwise appends " copy", " copy 2", …
fn available_connection_name(existing: &[SavedConnection], desired: &str) -> String {
    let desired = desired.trim();
    if !existing
        .iter()
        .any(|connection| connection.name.eq_ignore_ascii_case(desired))
    {
        return desired.to_string();
    }
    let mut index = 1;
    loop {
        let candidate = if index == 1 {
            format!("{desired} copy")
        } else {
            format!("{desired} copy {index}")
        };
        if !existing
            .iter()
            .any(|connection| connection.name.eq_ignore_ascii_case(&candidate))
        {
            return candidate;
        }
        index += 1;
    }
}

#[derive(Deserialize)]
struct ConnectionsFile {
    #[serde(default)]
    connections: Option<Vec<SavedConnection>>,
    #[serde(default)]
    connection: Option<SavedConnection>,
}

fn parse_connections_json(text: &str) -> Result<Vec<SavedConnection>, String> {
    if let Ok(connections) = serde_json::from_str::<Vec<SavedConnection>>(text) {
        return Ok(connections);
    }
    if let Ok(document) = serde_json::from_str::<ConnectionsFile>(text) {
        if let Some(connections) = document.connections {
            return Ok(connections);
        }
        if let Some(connection) = document.connection {
            return Ok(vec![connection]);
        }
    }
    if let Ok(connection) = serde_json::from_str::<SavedConnection>(text) {
        return Ok(vec![connection]);
    }
    Err("That file is not a Dataloom connections export.".to_string())
}

/// Native open panel for importing connections from JSON.
pub(crate) fn pick_connections_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Import connections")
        .add_filter("JSON files", &["json"])
        .pick_file()
}

/// Native open panel for picking the SQL file to import.
pub(crate) fn pick_sql_file(start: Option<PathBuf>) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new()
        .set_title("Choose an SQL file")
        .add_filter("SQL files", &["sql", "txt"]);
    if let Some(dir) = start {
        dialog = dialog.set_directory(dir);
    }
    dialog.pick_file()
}

/// Native open panel for choosing an SSL root certificate.
pub(crate) fn pick_certificate_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Choose a root certificate")
        .add_filter("Certificates", &["pem", "crt", "cer", "der"])
        .pick_file()
}

/// Native open panel for choosing an SSH private key.
pub(crate) fn pick_ssh_key_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Choose an SSH private key")
        .pick_file()
}

/// Native save panel for choosing where an SQL export is written.
pub(crate) fn save_sql_file(current: &str) -> Option<PathBuf> {
    let current = expand_path(current);
    let mut dialog = rfd::FileDialog::new()
        .set_title("Save SQL export")
        .add_filter("SQL files", &["sql"]);
    if let Some(parent) = current.parent().filter(|dir| dir.is_dir()) {
        dialog = dialog.set_directory(parent);
    }
    if let Some(name) = current.file_name() {
        dialog = dialog.set_file_name(name.to_string_lossy().to_string());
    }
    dialog.save_file()
}

/// Native save panel for a table export (`.sql`, `.csv`, `.xlsx`).
pub(crate) fn save_export_dialog(
    default_path: &Path,
    title: &str,
    filter_name: &str,
    extensions: &[&str],
) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new()
        .set_title(title)
        .add_filter(filter_name, extensions);
    if let Some(parent) = default_path.parent().filter(|dir| dir.is_dir()) {
        dialog = dialog.set_directory(parent);
    }
    if let Some(name) = default_path.file_name() {
        dialog = dialog.set_file_name(name.to_string_lossy().to_string());
    }
    let mut path = dialog.save_file()?;
    if path.extension().is_none() {
        if let Some(extension) = extensions.first() {
            path.set_extension(extension);
        }
    }
    Some(path)
}

/// Folder the browse dialogs start in: the typed path's folder, else Downloads.
pub(crate) fn browse_start_dir(text: &str) -> Option<PathBuf> {
    let path = expand_path(text);
    if path.is_dir() {
        return Some(path);
    }
    if let Some(parent) = path.parent().filter(|dir| dir.is_dir()) {
        return Some(parent.to_path_buf());
    }
    let home = std::env::var("HOME").ok()?;
    let downloads = PathBuf::from(home).join("Downloads");
    downloads.is_dir().then_some(downloads)
}

/// Expands a typed path: `~/Desktop/dump.sql` becomes the real home path.
pub(crate) fn expand_path(text: &str) -> PathBuf {
    let text = text.trim();
    if let Some(rest) = text.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_default();
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(text)
}

impl eframe::App for DataloomApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame_count += 1;
        // Remember which tab rendered this frame: switching tabs while a row
        // detail field has focus must still save the typed value.
        let active_title = self.tabs.get(self.active).map(|tab| tab.title.clone());

        // Apply a finished background query or import before drawing.
        self.poll_query_job(ctx);
        self.poll_import(ctx);

        // Install the native menu once the window exists, then drain its
        // events. Both run on the main thread, which AppKit requires.
        if self.native_menu.is_none() {
            self.native_menu = crate::menu::NativeMenu::install();
        }
        while let Some(command) = crate::menu::NativeMenu::poll() {
            self.dispatch_command(command, ctx);
        }

        // Dropping a data file on the window opens the import dialog.
        if self.import_dialog.is_none() && self.import_job.is_none() {
            let dropped: Vec<PathBuf> = ctx.input(|input| {
                input
                    .raw
                    .dropped_files
                    .iter()
                    .filter_map(|file| file.path.clone())
                    .collect()
            });
            if let Some(path) = dropped
                .into_iter()
                .find(|path| crate::import::format_for_path(path).is_some())
            {
                self.open_import_dialog(path);
            }
        }

        // A close request with unsaved SQL is intercepted once so the user can
        // save, discard or stay.
        if ctx.input(|input| input.viewport().close_requested()) && !self.allow_close {
            if self.settings.confirm_close
                && (self.has_unsaved_sql() || self.editor_tx.is_some())
            {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.close_confirm = true;
            } else {
                self.autosave_session(true);
            }
        }

        if self.screenshot.is_some() {
            ctx.request_repaint();
        }

        // macOS app shortcuts the window manager does not provide on its own.
        let quit = ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Q));
        let about = ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Comma));
        if quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if about {
            self.about_open = true;
        }

        let modal_open = self.new_connection.is_some()
            || self.db_tools.is_some()
            || self.row_editor.is_some()
            || self.column_editor.is_some()
            || self.column_delete.is_some()
            || self.row_delete.is_some()
            || self.connection_delete.is_some()
            || self.clone_table.is_some()
            || self.group_editor.is_some()
            || self.group_delete.is_some()
            || self.truncate_table.is_some()
            || self.delete_table.is_some()
            || self.palette_open
            || self.settings_open
            || self.value_editor.is_some()
            || self.params_dialog.is_some()
            || self.import_dialog.is_some()
            || self.import_job.is_some()
            || self.safe_query.is_some()
            || self.close_confirm;

        if !modal_open {
            let run = ctx.input(|i| {
                i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::Enter)
            });
            let run_all = ctx.input(|i| {
                i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::Enter)
            });
            let new_tab = ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::T));
            let new_conn = ctx.input(|i| {
                i.modifiers.command
                    && !i.modifiers.shift
                    && (i.key_pressed(egui::Key::K) || i.key_pressed(egui::Key::N))
            });
            let close_tab = ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::W));
            let save = ctx.input(|i| {
                i.modifiers.command
                    && !i.modifiers.shift
                    && i.key_pressed(egui::Key::S)
            });
            let find = ctx.input(|i| {
                i.modifiers.command
                    && !i.modifiers.shift
                    && i.key_pressed(egui::Key::F)
            });
            let palette = ctx.input(|i| {
                i.modifiers.command
                    && i.modifiers.shift
                    && i.key_pressed(egui::Key::P)
            });
            let explain = ctx.input(|i| {
                i.modifiers.command
                    && !i.modifiers.shift
                    && i.key_pressed(egui::Key::E)
            });
            let format = ctx.input(|i| {
                i.modifiers.command
                    && i.modifiers.shift
                    && i.key_pressed(egui::Key::F)
            });
            let theme = ctx.input(|i| {
                i.modifiers.command
                    && i.modifiers.shift
                    && i.key_pressed(egui::Key::D)
            });
            let zoom_in = ctx.input(|i| {
                i.modifiers.command
                    && !i.modifiers.shift
                    && (i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals))
            });
            let zoom_out = ctx.input(|i| {
                i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::Minus)
            });
            let zoom_reset = ctx.input(|i| {
                i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::Num0)
            });
            let toggle_read_only = ctx.input(|i| {
                i.modifiers.command
                    && i.modifiers.shift
                    && i.key_pressed(egui::Key::L)
            });
            if toggle_read_only {
                let next = !self.is_read_only();
                self.set_read_only(next);
            }
            if run || run_all {
                self.run_active();
            }
            if save {
                self.save_active();
            }
            if find {
                self.focus_search = true;
            }
            if palette {
                self.palette_open = true;
                self.palette_search.clear();
                self.palette_index = 0;
            }
            if explain {
                self.explain_active();
            }
            if format {
                self.format_active_sql();
            }
            if theme {
                self.toggle_theme(ctx);
            }
            if zoom_in {
                self.zoom_fonts(ctx, 1.0);
            } else if zoom_out {
                self.zoom_fonts(ctx, -1.0);
            } else if zoom_reset {
                self.reset_zoom(ctx);
            }
            if new_tab {
                self.new_query_tab(true);
            }
            if new_conn {
                self.open_new_connection_dialog();
            }
            if close_tab {
                self.close_tab(self.active);
            }
            // The grid is a focusable widget, so wants_keyboard_input() is
            // true after clicking a cell. Keep the workspace shortcuts alive
            // there, but not while a text field or the cell editor has focus.
            let grid_focused = match self.tabs.get(self.active).map(|tab| &tab.kind) {
                Some(TabKind::Table(_)) => Some(egui::Id::new(("table-grid", self.active))),
                Some(TabKind::Query(_)) => Some(egui::Id::new(("query-grid", self.active))),
                None => None,
            }
            .map(|id| {
                ctx.memory(|memory| memory.has_focus(crate::widgets::grid_focus_id(id)))
            })
            .unwrap_or(false);
            if !ctx.wants_keyboard_input() || grid_focused {
                let prev =
                    ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::ArrowLeft));
                let next =
                    ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::ArrowRight));
                if prev || next {
                    self.page_active(next);
                }
                // WorkspaceWindow.nib: "Toggle Row Detail (Space)".
                if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
                    self.toggle_row_detail();
                }
                let undo = ctx.input(|i| {
                    i.modifiers.command
                        && !i.modifiers.shift
                        && i.key_pressed(egui::Key::Z)
                });
                let redo = ctx.input(|i| {
                    i.modifiers.command
                        && i.modifiers.shift
                        && i.key_pressed(egui::Key::Z)
                });
                if undo {
                    self.undo_edit();
                } else if redo {
                    self.redo_edit();
                }
            }
        }

        self.topbar(ctx);
        self.sidebar(ctx);
        if self.live.is_some() {
            self.tabbar(ctx);
        }
        self.statusbar(ctx);
        self.row_detail_panel(ctx);
        self.central(ctx);
        if self.new_connection.is_some() {
            self.new_connection_dialog(ctx);
        }
        if self.row_editor.is_some() {
            self.row_editor_window(ctx);
        }
        if self.column_editor.is_some() {
            self.column_editor_window(ctx);
        }
        if self.column_delete.is_some() {
            self.column_delete_window(ctx);
        }
        if self.row_delete.is_some() {
            self.row_delete_window(ctx);
        }
        if self.connection_delete.is_some() {
            self.connection_delete_window(ctx);
        }
        if self.clone_table.is_some() {
            self.clone_table_window(ctx);
        }
        if self.group_editor.is_some() {
            self.group_editor_window(ctx);
        }
        if self.group_delete.is_some() {
            self.group_delete_window(ctx);
        }
        if self.truncate_table.is_some() {
            self.truncate_table_window(ctx);
        }
        if self.delete_table.is_some() {
            self.delete_table_window(ctx);
        }
        if self.history_open {
            self.history_window(ctx);
        }
        if self.db_tools.is_some() {
            self.db_tools_window(ctx);
        }
        if self.about_open {
            self.about_window(ctx);
        }
        if self.value_editor.is_some() {
            self.value_editor_window(ctx);
        }
        if self.import_dialog.is_some() {
            self.import_window(ctx);
        }
        if self.import_job.is_some() {
            self.import_progress_window(ctx);
        }
        if self.palette_open {
            self.command_palette_window(ctx);
        }
        if self.settings_open {
            self.settings_window(ctx);
        }
        if self.params_dialog.is_some() {
            self.params_window(ctx);
        }
        if self.safe_query.is_some() {
            self.safe_query_window(ctx);
        }
        if self.close_confirm {
            self.close_confirm_window(ctx);
        }
        self.toast_ui(ctx);

        if let Some(previous) = active_title {
            let current = self.tabs.get(self.active).map(|tab| tab.title.clone());
            if current.as_deref() != Some(previous.as_str()) {
                if let Some(idx) = self.tabs.iter().position(|tab| tab.title == previous) {
                    self.flush_row_detail_edits(idx);
                }
            }
        }

        if let Some(text) = self.pending_copy.take() {
            ctx.output_mut(|o| o.copied_text = text);
        }

        if self.running_query.is_some() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        if self.import_job.is_some() {
            ctx.request_repaint_after(Duration::from_millis(80));
        }

        self.autosave_session(false);

        if let Some(job) = &mut self.screenshot {
            if self.frame_count >= 120 && !job.requested {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                job.requested = true;
            }
            let shot = ctx.input(|i| {
                i.events.iter().find_map(|event| {
                    if let egui::Event::Screenshot { image, .. } = event {
                        Some(image.clone())
                    } else {
                        None
                    }
                })
            });
            if let Some(image) = shot {
                match crate::widgets::save_png(&image, &job.path) {
                    Ok(()) => std::process::exit(0),
                    Err(e) => {
                        eprintln!("Could not save screenshot: {e}");
                        std::process::exit(2);
                    }
                }
            }
            if self.frame_count > 900 {
                eprintln!("The window never produced a screenshot.");
                std::process::exit(3);
            }
            ctx.request_repaint();
        }
    }
}

// ----------------------------------------------------------------------
// Small helpers shared by the views
// ----------------------------------------------------------------------

pub(crate) fn default_new_path(name: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{home}/Documents/{}.db", slugify(name))
}

pub(crate) fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for c in name.chars().flat_map(|c| c.to_lowercase()) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "database".to_string()
    } else {
        trimmed
    }
}

pub(crate) fn validate_new_form(form: &NewConnection, create: bool) -> Result<(), String> {
    if form.name.trim().is_empty() {
        return Err("Give the connection a name.".to_string());
    }
    match form.driver {
        Driver::Sqlite => {
            if form.path.trim().is_empty() {
                return Err("Choose a file path.".to_string());
            }
            let path = Path::new(form.path.trim());
            if create {
                if path.exists() {
                    return Err(
                        "That file already exists. Use Open existing instead, or pick another name."
                            .to_string(),
                    );
                }
                if let Some(parent) = path.parent() {
                    if !parent.as_os_str().is_empty() {
                        std::fs::create_dir_all(parent)
                            .map_err(|e| format!("Could not create the folder: {e}"))?;
                    }
                }
                rusqlite::Connection::open(path)
                    .map_err(|e| format!("Could not create the database: {e}"))?;
            } else if !path.exists() {
                return Err(
                    "That file was not found. Check the path, or use Create & connect.".to_string(),
                );
            }
        }
        Driver::MySql | Driver::Postgres => {
            if form.host.trim().is_empty() {
                return Err("Enter the server host.".to_string());
            }
            if form.user.trim().is_empty() {
                return Err("Enter the user name.".to_string());
            }
            if form.port.trim().is_empty() {
                return Err("Enter the port.".to_string());
            }
            if form.port.trim().parse::<u16>().is_err() {
                return Err("The port must be a number between 1 and 65535.".to_string());
            }
            if form.database.trim().is_empty() {
                return Err("Enter the database name.".to_string());
            }
            let ssl_mode = form.ssl_mode.trim();
            if !matches!(ssl_mode, "" | "disable" | "require" | "verify-ca" | "verify-full") {
                return Err("Choose a valid SSL mode.".to_string());
            }
            if ssl_mode != "disable"
                && !form.ssl_root_cert.trim().is_empty()
                && !expand_path(&form.ssl_root_cert).is_file()
            {
                return Err("The SSL root certificate file was not found.".to_string());
            }
            if form.ssh_enabled {
                if form.ssh_host.trim().is_empty() {
                    return Err("Enter the SSH host.".to_string());
                }
                if form.ssh_port.trim().is_empty() {
                    return Err("Enter the SSH port.".to_string());
                }
                if form
                    .ssh_port
                    .trim()
                    .parse::<u16>()
                    .map(|port| port == 0)
                    .unwrap_or(true)
                {
                    return Err("The SSH port must be between 1 and 65535.".to_string());
                }
                if !form.ssh_key_path.trim().is_empty()
                    && !expand_path(&form.ssh_key_path).is_file()
                {
                    return Err("The SSH private key file was not found.".to_string());
                }
                if ssl_mode == "verify-full" {
                    return Err(
                        "TLS verify-full cannot check the certificate host name through an SSH tunnel; choose Verify CA."
                            .to_string(),
                    );
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection(name: &str, group: Option<u64>) -> SavedConnection {
        SavedConnection {
            id: 1,
            name: name.to_string(),
            path: String::new(),
            driver: "MySQL".to_string(),
            host: "127.0.0.1".to_string(),
            port: 3306,
            user: "root".to_string(),
            password: String::new(),
            database: "demo".to_string(),
                        read_only: false,
                        ssl_mode: "disable".to_string(),
            ssl_root_cert: String::new(),
            ssh_host: String::new(),
            ssh_port: 0,
            ssh_user: String::new(),
            ssh_key_path: String::new(),
            group_id: group,
        }
    }

    fn sqlite_saved(path: &Path) -> SavedConnection {
        SavedConnection {
            id: 1,
            name: "Worker test".to_string(),
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

    fn worker_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dataloom-worker-{}-{label}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run_worker_request(
        worker: &QueryWorker,
        id: u64,
        _kind: QueryKind,
        sql: &str,
        read_only: bool,
    ) -> db::QueryOutcome {
        let (control_tx, control_rx) = mpsc::channel();
        worker
            .tx
            .send(QueryRequest {
                id,
                sql: sql.to_string(),
                max_rows: 0,
                read_only,
                control: control_tx,
                cancelled: Arc::new(AtomicBool::new(false)),
            })
            .unwrap();
        // Drain the cancel handle the worker publishes before running.
        let _ = control_rx.recv_timeout(Duration::from_secs(5));
        let (_, outcome) = worker
            .result_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the worker did not answer");
        outcome
    }

    #[test]
    fn query_worker_keeps_a_transaction_across_requests() {
        let dir = worker_dir("transaction");
        let path = dir.join("tx.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE items (id INTEGER PRIMARY KEY, label TEXT);")
            .unwrap();
        drop(conn);

        let worker = QueryWorker::spawn(sqlite_saved(&path));
        let begin = run_worker_request(&worker, 1, QueryKind::Begin, "BEGIN", false);
        assert_eq!(begin.error, None);
        let insert = run_worker_request(
            &worker,
            2,
            QueryKind::Sql,
            "INSERT INTO items (label) VALUES ('rolled back')",
            false,
        );
        assert_eq!(insert.error, None);
        let rollback = run_worker_request(&worker, 3, QueryKind::Rollback, "ROLLBACK", false);
        assert_eq!(rollback.error, None);

        let conn = rusqlite::Connection::open(&path).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM items", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0, "ROLLBACK must undo the insert");

        run_worker_request(&worker, 4, QueryKind::Begin, "BEGIN", false);
        run_worker_request(
            &worker,
            5,
            QueryKind::Sql,
            "INSERT INTO items (label) VALUES ('kept')",
            false,
        );
        run_worker_request(&worker, 6, QueryKind::Commit, "COMMIT", false);
        let count: i64 = conn
            .query_row("SELECT count(*) FROM items", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "COMMIT must keep the insert");
        drop(worker);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn query_worker_applies_the_read_only_flag_per_request() {
        let dir = worker_dir("readonly");
        let path = dir.join("ro.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE items (id INTEGER PRIMARY KEY, label TEXT);
             INSERT INTO items (label) VALUES ('one');",
        )
        .unwrap();
        drop(conn);

        let worker = QueryWorker::spawn(sqlite_saved(&path));
        let blocked = run_worker_request(
            &worker,
            1,
            QueryKind::Sql,
            "UPDATE items SET label = 'two'",
            true,
        );
        assert!(blocked
            .error
            .unwrap_or_default()
            .contains("Read-only"));
        let read = run_worker_request(&worker, 2, QueryKind::Sql, "SELECT count(*) FROM items", true);
        assert_eq!(read.error, None);
        // Turning the flag off on a later request allows the write.
        let allowed = run_worker_request(
            &worker,
            3,
            QueryKind::Sql,
            "UPDATE items SET label = 'two'",
            false,
        );
        assert_eq!(allowed.error, None);
        drop(worker);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn query_worker_returns_results_from_its_own_thread() {
        let dir = worker_dir("result");
        let path = dir.join("worker.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT);
             INSERT INTO notes (body) VALUES ('hello');",
        )
        .unwrap();
        drop(conn);

        let worker = QueryWorker::spawn(sqlite_saved(&path));
        let (control_tx, _control_rx) = mpsc::channel();
        worker
            .tx
            .send(QueryRequest {
                id: 1,
                sql: "SELECT body FROM notes".to_string(),
                max_rows: 0,
                read_only: false,
                control: control_tx,
                cancelled: Arc::new(AtomicBool::new(false)),
            })
            .unwrap();
        let (id, outcome) = worker
            .result_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the worker did not answer");
        assert_eq!(id, 1);
        assert_eq!(outcome.error, None);
        assert_eq!(outcome.rows[0][0].as_deref(), Some("hello"));
        drop(worker);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sqlite_query_can_be_cancelled_through_the_worker() {
        let dir = worker_dir("cancel");
        let path = dir.join("cancel.db");
        rusqlite::Connection::open(&path).unwrap();

        let worker = QueryWorker::spawn(sqlite_saved(&path));
        let (control_tx, control_rx) = mpsc::channel();
        worker
            .tx
            .send(QueryRequest {
                id: 2,
                sql: "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM cnt WHERE x < 1000000000) SELECT count(*) FROM cnt".to_string(),
                max_rows: 0,
                read_only: false,
                control: control_tx,
                cancelled: Arc::new(AtomicBool::new(false)),
            })
            .unwrap();
        let handle = control_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the worker did not publish a cancel handle");
        // Let sqlite3_step() enter the recursive loop: an interrupt issued
        // before the statement starts is discarded by SQLite.
        thread::sleep(Duration::from_millis(100));
        handle.cancel().expect("SQLite interrupt failed");
        let (_, outcome) = worker
            .result_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the worker did not stop");
        assert!(
            outcome.error.is_some(),
            "a cancelled query must report an error"
        );
        drop(worker);
        std::fs::remove_dir_all(&dir).ok();
    }

    fn mysql_form() -> NewConnection {
        NewConnection {
            driver: Driver::MySql,
            name: "Test".to_string(),
            path: String::new(),
            path_touched: false,
            host: "db.example.com".to_string(),
            port: "3306".to_string(),
            user: "app".to_string(),
            password: String::new(),
            database: "shop".to_string(),
            read_only: false,
            ssl_mode: "disable".to_string(),
            ssl_root_cert: String::new(),
            ssh_enabled: false,
            ssh_host: String::new(),
            ssh_port: "22".to_string(),
            ssh_user: String::new(),
            ssh_key_path: String::new(),
            group_id: None,
            error: None,
            editing: None,
        }
    }

    #[test]
    fn connection_form_validates_tls_and_ssh() {
        let mut form = mysql_form();
        assert!(validate_new_form(&form, false).is_ok());

        form.ssh_enabled = true;
        assert!(validate_new_form(&form, false)
            .unwrap_err()
            .contains("SSH host"));
        form.ssh_host = "bastion.example.com".to_string();
        form.ssl_mode = "verify-full".to_string();
        assert!(validate_new_form(&form, false)
            .unwrap_err()
            .contains("Verify CA"));

        form.ssl_mode = "verify-ca".to_string();
        form.ssl_root_cert = "/definitely/not/here.pem".to_string();
        assert!(validate_new_form(&form, false)
            .unwrap_err()
            .contains("root certificate"));
        form.ssl_root_cert.clear();
        assert!(validate_new_form(&form, false).is_ok());
    }

    #[test]
    fn parses_connection_export_shapes() {
        let single = serde_json::to_string(&connection("Single", None)).unwrap();
        assert_eq!(parse_connections_json(&single).unwrap().len(), 1);

        let array = serde_json::to_string(&vec![
            connection("One", None),
            connection("Two", None),
        ])
        .unwrap();
        assert_eq!(parse_connections_json(&array).unwrap().len(), 2);

        let envelope = r#"{"version":1,"connections":[
            {"name":"A","driver":"SQLite","path":"/tmp/a.db"},
            {"name":"B","driver":"PostgreSQL","host":"db","database":"x"}
        ]}"#;
        assert_eq!(parse_connections_json(envelope).unwrap().len(), 2);

        assert!(parse_connections_json(r#"{"connections":[]}"#)
            .unwrap()
            .is_empty());

        let exported = serde_json::json!({
            "version": 1,
            "connection": connection("Exported", Some(4)),
        });
        let exported = serde_json::to_string(&exported).unwrap();
        let parsed = parse_connections_json(&exported).unwrap();
        assert_eq!(parsed[0].name, "Exported");
        assert_eq!(parsed[0].group_id, Some(4));

        assert!(parse_connections_json("not json").is_err());
    }

    #[test]
    fn duplicate_names_get_a_stable_suffix() {
        let existing = vec![connection("Demo", None), connection("Demo copy", None)];
        assert_eq!(available_connection_name(&existing, "Demo"), "Demo copy 2");
        assert_eq!(available_connection_name(&existing, "Other"), "Other");
        assert_eq!(available_connection_name(&existing, "  demo  "), "demo copy 2");
    }
}
