use crate::app::{
    default_new_path, expand_path, validate_new_form, ColumnDelete, ColumnEditor,
    ConnectionMenuAction, DataloomApp, DetailMatch, NewConnection, RowDelete, RowEditor, RowField,
    TableMode, ToastKind, ToolsTab,
};
use crate::db::{self, ConnectionGroup, Driver, SavedConnection};
use crate::theme::Palette;
use crate::widgets;
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Margin, Rect, RichText, Rounding, ScrollArea,
    Sense, Stroke, TextEdit, TextStyle, Ui, Vec2,
};
use std::path::PathBuf;
use std::time::Instant;

/// A pending in-cell edit, carried out of the borrowing closure so the
/// update can go through `self.live`.
struct CellCommit {
    tab: usize,
    table: db::TableId,
    column: String,
    key_column: String,
    key_value: String,
    /// `None` is SQL NULL; `Some("")` is an empty string.
    value: Option<String>,
    /// Loaded value, used to skip writes that would not change anything.
    original: Option<String>,
    row: usize,
    col: usize,
}

/// Value the in-cell editor should write. An untouched NULL stays NULL; an
/// empty editor for a non-NULL value writes the empty string explicitly.
pub(crate) fn edited_cell_value(original: &Option<String>, text: &str) -> Option<String> {
    if original.is_none() && text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// Builds a pending cell write. Returns `None` when the cell has no usable
/// key or when the new value equals the loaded one, so a no-op edit never
/// reaches the database (and never turns NULL into '').
fn cell_commit(
    tab: &crate::app::TableTab,
    tab_index: usize,
    row: usize,
    col: usize,
    value: Option<String>,
) -> Option<CellCommit> {
    let key_column = tab.data.key_column.clone()?;
    let key_value = tab.data.rowids.get(row).cloned().flatten()?;
    let column = tab.data.columns.get(col).cloned()?;
    let original = tab
        .data
        .rows
        .get(row)
        .and_then(|values| values.get(col))
        .cloned()
        .flatten();
    if value == original {
        return None;
    }
    Some(CellCommit {
        tab: tab_index,
        table: tab.table.clone(),
        column,
        key_column,
        key_value,
        value,
        original,
        row,
        col,
    })
}

/// Action picked from a row detail field's chevron menu. TablePlus exposes the
/// NULL/EMPTY conversions and copy from `RowDetailPopupButton`
/// (RightSidebarController.nib); a value of `None` is SQL NULL.
enum DetailFieldAction {
    SetNull,
    SetEmpty,
    Copy(String),
}

/// Actions selectable from a connection group header's right-click menu.
enum GroupMenuAction {
    Rename,
    Delete,
}

/// Queues a row detail write, replacing an earlier queued write for the same
/// field. A single frame can produce more than one commit (e.g. the focused
/// field loses focus while another field's chevron menu is used), and dropping
/// either one would silently lose a change.
fn queue_detail_commit(
    commits: &mut Vec<(String, Option<String>)>,
    column: String,
    value: Option<String>,
) {
    if let Some(index) = commits.iter().position(|(name, _)| name == &column) {
        commits.remove(index);
    }
    commits.push((column, value));
}

/// Matches a field name against the row detail search box. "Contains" is a
/// plain substring test; "Fuzzy - default" (the menu label in the original
/// nib) is an ordered subsequence match.
fn detail_field_matches(
    name: &str,
    needle: &str,
    mode: DetailMatch,
    case_sensitive: bool,
) -> bool {
    if needle.is_empty() {
        return true;
    }
    let fold = |text: &str| -> Vec<char> {
        if case_sensitive {
            text.chars().collect()
        } else {
            text.chars().flat_map(|c| c.to_lowercase()).collect()
        }
    };
    let name = fold(name);
    let needle = fold(needle);
    match mode {
        DetailMatch::Contains => name
            .windows(needle.len())
            .any(|window| window == needle.as_slice()),
        DetailMatch::Fuzzy => {
            let mut wanted = needle.iter().peekable();
            for c in name {
                if wanted.peek() == Some(&&c) {
                    wanted.next();
                }
            }
            wanted.peek().is_none()
        }
    }
}

const SSL_MODE_OPTIONS: &[(&str, &str)] = &[
    ("disable", "Disabled"),
    ("require", "Require (encrypt only)"),
    ("verify-ca", "Verify CA"),
    ("verify-full", "Verify full"),
];

fn ssl_mode_label(mode: &str) -> &'static str {
    SSL_MODE_OPTIONS
        .iter()
        .find(|(value, _)| *value == mode)
        .map(|(_, label)| *label)
        .unwrap_or("Disabled")
}

/// Rows whose any cell contains `needle` (case-insensitive). Used by the
/// result search, which filters in memory without touching the database.
fn filter_query_rows(rows: &[Vec<Option<String>>], needle: &str) -> Vec<Vec<Option<String>>> {
    rows.iter()
        .filter(|row| row_matches(row, needle))
        .cloned()
        .collect()
}

fn row_matches(row: &[Option<String>], needle: &str) -> bool {
    let needle = needle.to_ascii_lowercase();
    row.iter()
        .flatten()
        .any(|value| value.to_ascii_lowercase().contains(&needle))
}

pub(crate) fn offset_of_line(sql: &str, line: usize) -> usize {
    if line <= 1 {
        return 0;
    }
    sql.match_indices('\n')
        .nth(line - 2)
        .map(|(index, _)| index + 1)
        .unwrap_or(sql.len())
}

impl DataloomApp {
    /// Fuzzy-filtered list of every command the app can run.
    fn palette_commands(&self) -> Vec<crate::menu::CommandId> {
        use crate::menu::CommandId;
        let query_active = matches!(
            self.tabs.get(self.active).map(|tab| &tab.kind),
            Some(crate::app::TabKind::Query(_))
        );
        let needle = self.palette_search.trim().to_ascii_lowercase();
        CommandId::ALL
            .iter()
            .copied()
            .filter(|command| {
                if command.needs_connection() && self.live.is_none() {
                    return false;
                }
                if command.needs_query_tab() && !query_active {
                    return false;
                }
                if needle.is_empty() {
                    return true;
                }
                command.label().to_ascii_lowercase().contains(&needle)
                    || command
                        .shortcut()
                        .to_ascii_lowercase()
                        .contains(&needle)
            })
            .collect()
    }

    pub(crate) fn command_palette_window(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let commands = self.palette_commands();
        let mut chosen: Option<crate::menu::CommandId> = None;
        let mut close = false;
        if !commands.is_empty() {
            self.palette_index = self.palette_index.min(commands.len() - 1);
        }
        egui::Window::new("Command palette")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 90.0))
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(Rounding::same(10.0))
                    .inner_margin(Margin::symmetric(14.0, 12.0)),
            )
            .show(ctx, |ui| {
                ui.set_width(460.0);
                let search = ui.add_sized(
                    Vec2::new(ui.available_width(), 30.0),
                    TextEdit::singleline(&mut self.palette_search)
                        .id(egui::Id::new("palette-search"))
                        .hint_text("Type a command…")
                        .font(FontId::proportional(13.0)),
                );
                if search.has_focus() || search.lost_focus() {
                    search.request_focus();
                }
                ui.add_space(6.0);
                let down = ui.input_mut(|i| {
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)
                });
                let up = ui.input_mut(|i| {
                    i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)
                });
                if down && !commands.is_empty() {
                    self.palette_index = (self.palette_index + 1) % commands.len();
                } else if up && !commands.is_empty() {
                    self.palette_index = self
                        .palette_index
                        .checked_sub(1)
                        .unwrap_or(commands.len() - 1);
                }
                if ui.input_mut(|i| {
                    i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                }) {
                    chosen = commands.get(self.palette_index).copied();
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
                ScrollArea::vertical()
                    .id_salt("palette-list")
                    .max_height(330.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if commands.is_empty() {
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new("No matching commands")
                                    .size(11.4)
                                    .color(p.faint),
                            );
                        }
                        for (index, command) in commands.iter().enumerate() {
                            let selected = index == self.palette_index;
                            let response = ui.add_sized(
                                Vec2::new(ui.available_width(), 26.0),
                                egui::Button::new(
                                    RichText::new(command.label())
                                        .size(12.4)
                                        .color(if selected { p.text } else { p.muted }),
                                )
                                .fill(if selected { p.selection } else { Color32::TRANSPARENT })
                                .stroke(Stroke::NONE),
                            );
                            if !command.shortcut().is_empty() {
                                ui.painter().text(
                                    egui::pos2(
                                        response.rect.right() - 8.0,
                                        response.rect.center().y,
                                    ),
                                    Align2::RIGHT_CENTER,
                                    command.shortcut(),
                                    FontId::proportional(10.6),
                                    p.faint,
                                );
                            }
                            if response.clicked() {
                                chosen = Some(*command);
                            }
                        }
                    });
            });
        if let Some(command) = chosen {
            self.palette_open = false;
            self.dispatch_command(command, ctx);
        } else {
            self.palette_open = !close;
        }
    }

    pub(crate) fn settings_window(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let mut open = self.settings_open;
        let mut changed = false;
        let mut reset = false;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(Rounding::same(10.0))
                    .inner_margin(Margin::symmetric(18.0, 16.0)),
            )
            .show(ctx, |ui| {
                ui.set_width(420.0);
                ui.label(
                    RichText::new("Appearance")
                        .size(10.6)
                        .strong()
                        .color(p.faint),
                );
                ui.add_space(6.0);
                if ui
                    .checkbox(&mut self.settings.dark, "Dark theme")
                    .changed()
                {
                    changed = true;
                }
                egui::Grid::new("settings-grid")
                    .num_columns(2)
                    .spacing(Vec2::new(14.0, 8.0))
                    .show(ui, |ui| {
                        ui.label(RichText::new("Data font").size(11.4).color(p.muted));
                        egui::ComboBox::from_id_salt("settings-data-font")
                            .selected_text(
                                RichText::new(format!("{} pt", self.settings.data_font_size))
                                    .size(11.4),
                            )
                            .show_ui(ui, |ui| {
                                for (value, label) in crate::settings::FONT_SIZE_OPTIONS {
                                    if ui
                                        .selectable_value(
                                            &mut self.settings.data_font_size,
                                            *value,
                                            *label,
                                        )
                                        .changed()
                                    {
                                        changed = true;
                                    }
                                }
                            });
                        ui.end_row();
                        ui.label(RichText::new("SQL font").size(11.4).color(p.muted));
                        egui::ComboBox::from_id_salt("settings-sql-font")
                            .selected_text(
                                RichText::new(format!("{} pt", self.settings.sql_font_size))
                                    .size(11.4),
                            )
                            .show_ui(ui, |ui| {
                                for (value, label) in crate::settings::FONT_SIZE_OPTIONS {
                                    if ui
                                        .selectable_value(
                                            &mut self.settings.sql_font_size,
                                            *value,
                                            *label,
                                        )
                                        .changed()
                                    {
                                        changed = true;
                                    }
                                }
                            });
                        ui.end_row();
                        ui.label(RichText::new("Row height").size(11.4).color(p.muted));
                        egui::ComboBox::from_id_salt("settings-row-height")
                            .selected_text(
                                RichText::new(format!("{} pt", self.settings.row_height))
                                    .size(11.4),
                            )
                            .show_ui(ui, |ui| {
                                for (value, label) in crate::settings::ROW_HEIGHT_OPTIONS {
                                    if ui
                                        .selectable_value(
                                            &mut self.settings.row_height,
                                            *value,
                                            *label,
                                        )
                                        .changed()
                                    {
                                        changed = true;
                                    }
                                }
                            });
                        ui.end_row();
                        ui.label(RichText::new("Tab width").size(11.4).color(p.muted));
                        egui::ComboBox::from_id_salt("settings-tab-width")
                            .selected_text(RichText::new(self.settings.tab_width_label()).size(11.4))
                            .show_ui(ui, |ui| {
                                for width in crate::theme::TAB_WIDTH_OPTIONS {
                                    if ui
                                        .selectable_value(
                                            &mut self.settings.tab_width,
                                            width,
                                            format!("{width} spaces"),
                                        )
                                        .changed()
                                    {
                                        changed = true;
                                    }
                                }
                            });
                        ui.end_row();
                    });
                if ui
                    .checkbox(&mut self.settings.alternate_rows, "Alternating row colors")
                    .changed()
                {
                    changed = true;
                }
                if ui
                    .checkbox(&mut self.settings.show_row_numbers, "Show row numbers")
                    .changed()
                {
                    changed = true;
                }
                ui.add_space(10.0);
                ui.label(
                    RichText::new("Session")
                        .size(10.6)
                        .strong()
                        .color(p.faint),
                );
                ui.add_space(4.0);
                if ui
                    .checkbox(
                        &mut self.settings.restore_session,
                        "Reopen the last connection and tabs on launch",
                    )
                    .changed()
                {
                    changed = true;
                }
                if ui
                    .checkbox(
                        &mut self.settings.confirm_close,
                        "Ask before closing with unsaved SQL",
                    )
                    .changed()
                {
                    changed = true;
                }
                if ui
                    .checkbox(
                        &mut self.settings.safe_mode,
                        "Safe mode: confirm before running statements that write",
                    )
                    .changed()
                {
                    changed = true;
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Reset to defaults").size(11.4).color(p.muted),
                            )
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::NONE),
                        )
                        .clicked()
                    {
                        reset = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new("Settings are saved automatically.")
                                .size(10.0)
                                .color(p.faint),
                        );
                    });
                });
            });
        if reset {
            let dark = self.settings.dark;
            self.settings = crate::settings::Settings {
                dark,
                ..crate::settings::Settings::default()
            };
            changed = true;
        }
        self.settings_open = open;
        if changed {
            self.apply_settings(ctx);
        }
    }

    /// Asks for the values of `:name` placeholders before running.
    pub(crate) fn params_window(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let Some(mut dialog) = self.params_dialog.take() else {
            return;
        };
        let mut run = false;
        let mut close = false;
        egui::Window::new("Query parameters")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(Rounding::same(10.0))
                    .inner_margin(Margin::symmetric(18.0, 16.0)),
            )
            .show(ctx, |ui| {
                ui.set_width(400.0);
                ui.label(
                    RichText::new(
                        "This statement uses named parameters. Values are substituted as SQL literals.",
                    )
                    .size(11.2)
                    .color(p.muted),
                );
                ui.add_space(8.0);
                for (index, name) in dialog.names.iter().enumerate() {
                    ui.label(
                        RichText::new(format!(":{name}"))
                            .monospace()
                            .size(11.2)
                            .color(p.code),
                    );
                    ui.add_sized(
                        Vec2::new(ui.available_width(), 26.0),
                        TextEdit::singleline(&mut dialog.values[index])
                            .hint_text("value"),
                    );
                    ui.add_space(6.0);
                }
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let primary = egui::Button::new(
                        RichText::new("Run").size(12.0).color(Color32::WHITE),
                    )
                    .fill(p.accent)
                    .rounding(Rounding::same(6.0))
                    .min_size(Vec2::new(90.0, 27.0));
                    if ui.add(primary).clicked() {
                        run = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Cancel").size(12.0).color(p.muted))
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE),
                        )
                        .clicked()
                    {
                        close = true;
                    }
                });
            });
        if run {
            self.params_dialog = Some(dialog);
            self.run_active();
        } else if !close {
            self.params_dialog = Some(dialog);
        }
    }

    pub(crate) fn close_confirm_window(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let mut save = false;
        let mut discard = false;
        let mut cancel = false;
        egui::Window::new("Unsaved SQL")
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
                ui.set_width(380.0);
                ui.label(
                    RichText::new("The query editor has changes that were not run.")
                        .size(12.4)
                        .color(p.text),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Saving keeps the session so the tabs reopen next launch. Discarding clears the saved session.",
                    )
                    .size(10.8)
                    .color(p.muted),
                );
                if self.editor_tx.is_some() {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(
                            "An open transaction will be rolled back when the app quits.",
                        )
                        .size(10.6)
                        .color(p.amber),
                    );
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    let primary = egui::Button::new(
                        RichText::new("Save & quit").size(12.0).color(Color32::WHITE),
                    )
                    .fill(p.accent)
                    .rounding(Rounding::same(6.0))
                    .min_size(Vec2::new(110.0, 28.0));
                    if ui.add(primary).clicked() {
                        save = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Discard").size(12.0).color(p.red),
                            )
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::new(1.0_f32, p.border))
                            .rounding(Rounding::same(6.0)),
                        )
                        .clicked()
                    {
                        discard = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Cancel").size(12.0).color(p.muted))
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE),
                        )
                        .clicked()
                    {
                        cancel = true;
                    }
                });
            });
        if save {
            for tab in &mut self.tabs {
                if let crate::app::TabKind::Query(query) = &mut tab.kind {
                    query.dirty = false;
                }
            }
            self.close_confirm = false;
            self.allow_close = true;
            self.autosave_session(true);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if discard {
            crate::settings::Session::clear();
            self.close_confirm = false;
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if cancel {
            self.close_confirm = false;
        }
    }
}

impl DataloomApp {
    /// Import dialog: source options, column mapping, preview and target.
    pub(crate) fn import_window(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let Some(mut dialog) = self.import_dialog.take() else {
            return;
        };
        let tables: Vec<db::TableId> = self
            .live
            .as_ref()
            .map(|live| live.tables.iter().map(db::TableInfo::id).collect())
            .unwrap_or_default();
        let mut close = false;
        let mut start = false;
        let mut reload = false;
        let mut new_path: Option<PathBuf> = None;
        let delimiter_before = dialog.delimiter;
        let header_before = dialog.source.has_header;
        let sheet_before = dialog.sheet;
        let mode_before = dialog.mode;
        let existing_before = dialog.existing.clone();
        let file_name = dialog
            .path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| dialog.path.display().to_string());
        let row_count = dialog.source.rows.len();
        let source_columns = dialog.source.columns.clone();
        let samples: Vec<String> = (0..source_columns.len())
            .map(|index| dialog.source.sample(index, 3))
            .collect();

        egui::Window::new("Import data")
            .collapsible(false)
            .resizable(true)
            .default_size(Vec2::new(820.0, 620.0))
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(Rounding::same(10.0))
                    .inner_margin(Margin::symmetric(18.0, 16.0)),
            )
            .show(ctx, |ui| {
                ui.set_width(780.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&file_name).size(13.0).strong().color(p.text));
                    ui.label(
                        RichText::new(dialog.format.label())
                            .size(10.4)
                            .color(p.faint),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("Close").size(11.4).color(p.muted),
                                )
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE),
                            )
                            .clicked()
                        {
                            close = true;
                        }
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("Choose file…").size(11.4).color(p.text),
                                )
                                .fill(p.card)
                                .stroke(Stroke::new(1.0_f32, p.border)),
                            )
                            .clicked()
                        {
                            new_path = crate::import::pick_data_file();
                        }
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("Reload").size(11.4).color(p.text),
                                )
                                .fill(p.card)
                                .stroke(Stroke::new(1.0_f32, p.border)),
                            )
                            .clicked()
                        {
                            reload = true;
                        }
                    });
                });
                ui.add_space(2.0);
                ui.label(
                    RichText::new(dialog.path.display().to_string())
                        .monospace()
                        .size(9.8)
                        .color(p.faint),
                );
                ui.add_space(8.0);

                        // ---- source options ---------------------------------
                        ui.horizontal(|ui| {
                            match dialog.format {
                                crate::import::SourceFormat::Csv => {
                                    ui.label(
                                        RichText::new("Delimiter").size(10.8).color(p.muted),
                                    );
                                    egui::ComboBox::from_id_salt("import-delimiter")
                                        .selected_text(crate::import::delimiter_name(
                                            dialog.delimiter,
                                        ))
                                        .width(110.0)
                                        .show_ui(ui, |ui| {
                                            for (value, label) in [
                                                (',', "Comma"),
                                                ('\t', "Tab"),
                                                (';', "Semicolon"),
                                                ('|', "Pipe"),
                                            ] {
                                                ui.selectable_value(
                                                    &mut dialog.delimiter,
                                                    value,
                                                    label,
                                                );
                                            }
                                        });
                                }
                                crate::import::SourceFormat::Excel => {
                                    ui.label(
                                        RichText::new("Sheet").size(10.8).color(p.muted),
                                    );
                                    let current = dialog
                                        .source
                                        .sheets
                                        .get(dialog.sheet)
                                        .cloned()
                                        .unwrap_or_else(|| "Sheet 1".to_string());
                                    egui::ComboBox::from_id_salt("import-sheet")
                                        .selected_text(current)
                                        .width(170.0)
                                        .show_ui(ui, |ui| {
                                            for (index, sheet) in
                                                dialog.source.sheets.iter().enumerate()
                                            {
                                                ui.selectable_value(
                                                    &mut dialog.sheet,
                                                    index,
                                                    sheet,
                                                );
                                            }
                                        });
                                }
                                crate::import::SourceFormat::Json => {
                                    ui.label(
                                        RichText::new(
                                            "Array of objects, array of arrays or a single object",
                                        )
                                        .size(10.8)
                                        .color(p.faint),
                                    );
                                }
                            }
                            if dialog.format != crate::import::SourceFormat::Json {
                                ui.add_space(6.0);
                                ui.checkbox(
                                    &mut dialog.source.has_header,
                                    RichText::new("First row is a header")
                                        .size(11.0)
                                        .color(p.text),
                                );
                            }
                        });
                        if !dialog.source.warnings.is_empty() {
                            ui.add_space(3.0);
                            for warning in &dialog.source.warnings {
                                ui.label(RichText::new(warning).size(10.2).color(p.amber));
                            }
                        }
                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(8.0);

                        // ---- target -----------------------------------------
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Import into").size(10.8).color(p.muted),
                            );
                            if widgets::segmented(
                                ui,
                                &p,
                                "New table",
                                dialog.mode == crate::import::ImportMode::NewTable,
                            )
                            .clicked()
                            {
                                dialog.mode = crate::import::ImportMode::NewTable;
                            }
                            if widgets::segmented(
                                ui,
                                &p,
                                "Existing table",
                                dialog.mode == crate::import::ImportMode::Existing,
                            )
                            .clicked()
                            {
                                dialog.mode = crate::import::ImportMode::Existing;
                            }
                        });
                        ui.add_space(6.0);
                        match dialog.mode {
                            crate::import::ImportMode::NewTable => {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new("Table name").size(10.8).color(p.muted),
                                    );
                                    ui.add_sized(
                                        Vec2::new(240.0, 25.0),
                                        TextEdit::singleline(&mut dialog.new_table)
                                            .hint_text("imported_data"),
                                    );
                                    ui.checkbox(
                                        &mut dialog.trim,
                                        RichText::new("Trim spaces").size(10.8).color(p.text),
                                    );
                                });
                            }
                            crate::import::ImportMode::Existing => {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new("Table").size(10.8).color(p.muted),
                                    );
                                    let current = dialog
                                        .existing
                                        .as_ref()
                                        .map(|table| table.display())
                                        .unwrap_or_else(|| "Choose…".to_string());
                                    egui::ComboBox::from_id_salt("import-existing")
                                        .selected_text(current)
                                        .width(220.0)
                                        .show_ui(ui, |ui| {
                                            for table in &tables {
                                                let selected =
                                                    dialog.existing.as_ref() == Some(table);
                                                if ui
                                                    .selectable_label(
                                                        selected,
                                                        table.display(),
                                                    )
                                                    .clicked()
                                                {
                                                    dialog.existing = Some(table.clone());
                                                }
                                            }
                                        });
                                    ui.add_space(6.0);
                                    ui.checkbox(
                                        &mut dialog.truncate,
                                        RichText::new("Delete existing rows first")
                                            .size(10.8)
                                            .color(p.text),
                                    );
                                    ui.checkbox(
                                        &mut dialog.trim,
                                        RichText::new("Trim spaces").size(10.8).color(p.text),
                                    );
                                });
                            }
                        }
                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(8.0);

                        // ---- mapping ----------------------------------------
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Column mapping")
                                    .size(10.8)
                                    .strong()
                                    .color(p.faint),
                            );
                            ui.label(
                                RichText::new(
                                    "Skip a source column or point it at another target column.",
                                )
                                .size(10.2)
                                .color(p.faint),
                            );
                        });
                        ui.add_space(4.0);
                        ScrollArea::vertical()
                            .id_salt("import-map-scroll")
                            .max_height(210.0)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                        egui::Grid::new("import-mapping")
                            .num_columns(3)
                            .spacing(Vec2::new(12.0, 5.0))
                            .striped(true)
                            .show(ui, |ui| {
                                ui.label(RichText::new("Source").size(10.2).color(p.faint));
                                ui.label(RichText::new("Sample").size(10.2).color(p.faint));
                                ui.label(RichText::new("Target").size(10.2).color(p.faint));
                                ui.end_row();
                                for index in 0..source_columns.len() {
                                    ui.label(
                                        RichText::new(&source_columns[index])
                                            .size(11.0)
                                            .color(p.text),
                                    );
                                    ui.label(
                                        RichText::new(widgets::truncate(
                                            samples.get(index).map(String::as_str).unwrap_or(""),
                                            54,
                                        ))
                                        .size(10.2)
                                        .color(p.faint),
                                    );
                                    let selected = dialog
                                        .mapping
                                        .get(index)
                                        .copied()
                                        .flatten()
                                        .and_then(|target| dialog.targets.get(target))
                                        .map(|target| target.name.clone())
                                        .unwrap_or_else(|| "Skip".to_string());
                                    let unmapped = dialog
                                        .mapping
                                        .get(index)
                                        .copied()
                                        .flatten()
                                        .is_none();
                                    egui::ComboBox::from_id_salt(("import-map", index))
                                        .selected_text(
                                            RichText::new(selected).size(10.8).color(
                                                if unmapped { p.faint } else { p.text },
                                            ),
                                        )
                                        .width(210.0)
                                        .show_ui(ui, |ui| {
                                            if ui
                                                .selectable_label(unmapped, "Skip")
                                                .clicked()
                                            {
                                                if index < dialog.mapping.len() {
                                                    dialog.mapping[index] = None;
                                                }
                                            }
                                            for (target_index, target) in
                                                dialog.targets.iter().enumerate()
                                            {
                                                let chosen = dialog.mapping.get(index).copied()
                                                    == Some(Some(target_index));
                                                if ui
                                                    .selectable_label(chosen, &target.name)
                                                    .clicked()
                                                {
                                                    if index < dialog.mapping.len() {
                                                        dialog.mapping[index] =
                                                            Some(target_index);
                                                    }
                                                }
                                            }
                                        });
                                    ui.end_row();
                                }
                            });
                            });

                        // ---- options and preview ----------------------------
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("NULL values").size(10.8).color(p.muted));
                            ui.add_sized(
                                Vec2::new(150.0, 23.0),
                                TextEdit::singleline(&mut dialog.null_tokens)
                                    .hint_text("NULL, \\N"),
                            );
                            ui.add_space(6.0);
                            ui.checkbox(
                                &mut dialog.empty_as_null,
                                RichText::new("Empty string is NULL").size(10.8).color(p.text),
                            );
                            ui.add_space(6.0);
                            ui.label(RichText::new("Batch").size(10.8).color(p.muted));
                            ui.add(
                                egui::DragValue::new(&mut dialog.batch_size)
                                    .range(1..=1000)
                                    .speed(10.0),
                            );
                        });
                        ui.add_space(8.0);
                        let mut keep: Vec<(usize, usize)> = dialog
                            .mapping
                            .iter()
                            .enumerate()
                            .filter_map(|(source, target)| target.map(|target| (target, source)))
                            .collect();
                        keep.sort_by_key(|(target, _)| *target);
                        if !keep.is_empty() && !dialog.source.rows.is_empty() {
                            ui.label(
                                RichText::new(format!(
                                    "Preview · first {} rows",
                                    dialog.source.rows.len().min(4)
                                ))
                                .size(10.2)
                                .strong()
                                .color(p.faint),
                            );
                            ui.add_space(3.0);
                            egui::Grid::new("import-preview")
                                .num_columns(keep.len())
                                .spacing(Vec2::new(10.0, 3.0))
                                .striped(true)
                                .show(ui, |ui| {
                                    for (target, _) in &keep {
                                        let name = dialog
                                            .targets
                                            .get(*target)
                                            .map(|target| target.name.as_str())
                                            .unwrap_or("");
                                        ui.label(
                                            RichText::new(name).size(9.8).color(p.muted),
                                        );
                                    }
                                    ui.end_row();
                                    for row in dialog.source.rows.iter().take(4) {
                                        for (_, source) in &keep {
                                            let value = row
                                                .get(*source)
                                                .cloned()
                                                .flatten()
                                                .unwrap_or_else(|| "NULL".to_string());
                                            ui.label(
                                                RichText::new(widgets::truncate(&value, 20))
                                                    .monospace()
                                                    .size(9.6)
                                                    .color(if row
                                                        .get(*source)
                                                        .cloned()
                                                        .flatten()
                                                        .is_none()
                                                    {
                                                        p.faint
                                                    } else {
                                                        p.text
                                                    }),
                                            );
                                        }
                                        ui.end_row();
                                    }
                                });
                        }
                if let Some(error) = &dialog.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(error).size(11.0).color(p.red));
                }
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);
                let mapped = dialog.mapping.iter().flatten().count();
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{row_count} rows · {mapped} of {} source columns mapped",
                            source_columns.len()
                        ))
                        .size(10.4)
                        .color(p.faint),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let primary = egui::Button::new(
                            RichText::new("Import")
                                .size(12.0)
                                .color(Color32::WHITE),
                        )
                        .fill(p.accent)
                        .rounding(Rounding::same(6.0))
                        .min_size(Vec2::new(100.0, 28.0));
                        if ui
                            .add_enabled(
                                mapped > 0 && row_count > 0,
                                primary,
                            )
                            .clicked()
                        {
                            start = true;
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
                            close = true;
                        }
                    });
                });
            });

        if let Some(path) = new_path {
            self.open_import_dialog(path);
            return;
        }
        if reload {
            self.reload_import_source(&mut dialog);
        } else if dialog.delimiter != delimiter_before
            || dialog.source.has_header != header_before
            || dialog.sheet != sheet_before
        {
            self.reload_import_source(&mut dialog);
        } else if dialog.mode != mode_before || dialog.existing != existing_before {
            self.refresh_import_targets(&mut dialog);
        }
        if start {
            self.begin_import(dialog);
        } else if close {
            self.import_dialog = None;
        } else {
            self.import_dialog = Some(dialog);
        }
    }

    /// Progress overlay while the background import runs.
    pub(crate) fn import_progress_window(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let Some(job) = &self.import_job else {
            return;
        };
        let fraction = if job.total == 0 {
            0.0
        } else {
            (job.done as f32 / job.total as f32).clamp(0.0, 1.0)
        };
        let mut cancel = false;
        egui::Window::new("Importing data")
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
                ui.set_width(360.0);
                ui.label(
                    RichText::new(format!("Into {}", job.table_label))
                        .size(12.4)
                        .strong()
                        .color(p.text),
                );
                ui.add_space(8.0);
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .show_percentage()
                        .text(format!("{} / {} rows", job.done, job.total)),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("{:.1}s elapsed", job.started.elapsed().as_secs_f32()))
                        .size(10.2)
                        .color(p.faint),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Cancel import").size(11.6).color(p.red),
                            )
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::new(1.0_f32, p.border))
                            .rounding(Rounding::same(6.0)),
                        )
                        .clicked()
                    {
                        cancel = true;
                    }
                    ui.label(
                        RichText::new("Rows are committed in one transaction.")
                            .size(9.8)
                            .color(p.faint),
                    );
                });
            });
        if cancel {
            self.cancel_import();
        }
    }
}

/// The editor for one table column, from the structure metadata.
fn column_editor_kind(tt: &crate::app::TableTab, col: usize) -> db::FieldEditor {
    tt.data
        .columns
        .get(col)
        .and_then(|name| tt.structure.iter().find(|column| &column.name == name))
        .map(|column| db::FieldEditor::from_decl(&column.decl_type, &column.options))
        .unwrap_or(db::FieldEditor::Text)
}

fn fmt_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn fmt_backup_time(secs: u64) -> String {
    let (year, month, day, hour, minute, _) = db::utc_parts(secs as i64);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02} UTC")
}

impl DataloomApp {
    pub(crate) fn query_tab_ui(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let idx = self.active;
        let mut run_requested = false;
        let mut cancel_requested = false;
        let mut copy_text: Option<String> = None;
        let mut open_history = false;
        let mut chosen_limit: Option<usize> = None;
        let mut format_requested = false;
        let mut explain_requested = false;
        let mut template_requested: Option<&'static str> = None;
        let mut jump_to_error = false;
        let mut begin_tx = false;
        let mut commit_tx = false;
        let mut rollback_tx = false;
        let tx_state = self
            .editor_tx
            .as_ref()
            .map(|tx| (tx.started.elapsed(), tx.statements));
        let focus_search = self.focus_search;
        let completion_sources = self.completion_sources();
        let tab_width = self.settings.tab_width;
        let active_tab_id = self.tabs.get(idx).map(|tab| tab.id);
        let grid_style = widgets::GridStyle {
            row_height: self.settings.row_height,
            font_size: self.settings.data_font_size,
            alternate_rows: self.settings.alternate_rows,
            show_row_numbers: self.settings.show_row_numbers,
        };
        let running = self
            .tabs
            .get(idx)
            .and_then(|tab| match &tab.kind {
                crate::app::TabKind::Query(q) => Some(q.running),
                _ => None,
            })
            .unwrap_or(false);
        let any_running = self.running_query.is_some();
        let cancelling = self
            .running_query
            .as_ref()
            .map(|job| Some(job.tab_id) == active_tab_id && job.cancelled)
            .unwrap_or(false);
        let current_limit = self
            .tabs
            .get(idx)
            .and_then(|tab| match &tab.kind {
                crate::app::TabKind::Query(q) => Some(q.row_limit),
                _ => None,
            })
            .unwrap_or(db::DEFAULT_ROW_LIMIT);

        let avail_h = ui.available_height();
        let split = self
            .tabs
            .get(idx)
            .and_then(|tab| match &tab.kind {
                crate::app::TabKind::Query(q) => Some(q.split),
                _ => None,
            })
            .unwrap_or(self.settings.editor_split);
        let editor_h = (avail_h * split).clamp(150.0, (avail_h - 170.0).max(150.0));

        ui.allocate_ui_with_layout(
            Vec2::new(ui.available_width(), editor_h),
            Layout::top_down(Align::Min),
            |ui| {
                egui::Frame::none()
                    .fill(p.editor)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(Rounding::same(8.0))
                    .show(ui, |ui| {
                        ui.allocate_ui_with_layout(
                            Vec2::new(ui.available_width(), 32.0),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                ui.add_space(12.0);
                                ui.label(
                                    RichText::new("SQL EDITOR")
                                        .size(10.3)
                                        .strong()
                                        .color(p.faint),
                                );
                                ui.add_space(8.0);
                                ui.label(RichText::new("Dataloom.sql").size(10.5).color(p.faint));
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.add_space(10.0);
                                    if running {
                                        let cancel = egui::Button::new(
                                            RichText::new("Cancel")
                                                .size(10.7)
                                                .color(if cancelling { p.faint } else { p.red }),
                                        )
                                        .fill(p.card)
                                        .stroke(Stroke::new(1.0_f32, p.border))
                                        .rounding(Rounding::same(5.0));
                                        if ui.add_enabled(!cancelling, cancel).clicked() {
                                            cancel_requested = true;
                                        }
                                        ui.add(egui::Spinner::new());
                                        ui.label(
                                            RichText::new(if cancelling {
                                                "Cancelling…"
                                            } else {
                                                "Running…"
                                            })
                                            .size(10.6)
                                            .color(p.accent),
                                        );
                                    } else {
                                        let hint = egui::Button::new(
                                            RichText::new("Cmd+Enter").size(10.7).color(p.muted),
                                        )
                                        .fill(p.card)
                                        .stroke(Stroke::new(1.0_f32, p.border))
                                        .rounding(Rounding::same(5.0));
                                        if ui.add_enabled(!any_running, hint).clicked() {
                                            run_requested = true;
                                        }
                                        ui.label(RichText::new("Run").size(10.7).color(p.faint));
                                    }
                                    if let Some((elapsed, statements)) = tx_state {
                                        let commit = egui::Button::new(
                                            RichText::new("Commit")
                                                .size(10.4)
                                                .color(p.green),
                                        )
                                        .fill(p.card)
                                        .stroke(Stroke::new(1.0_f32, p.border))
                                        .rounding(Rounding::same(5.0));
                                        if ui
                                            .add_enabled(!any_running, commit)
                                            .on_hover_text("COMMIT the editor transaction")
                                            .clicked()
                                        {
                                            commit_tx = true;
                                        }
                                        let rollback = egui::Button::new(
                                            RichText::new("Rollback")
                                                .size(10.4)
                                                .color(p.red),
                                        )
                                        .fill(Color32::TRANSPARENT)
                                        .stroke(Stroke::new(1.0_f32, p.border))
                                        .rounding(Rounding::same(5.0));
                                        if ui
                                            .add_enabled(!any_running, rollback)
                                            .on_hover_text("ROLLBACK the editor transaction")
                                            .clicked()
                                        {
                                            rollback_tx = true;
                                        }
                                        crate::widgets::dot(ui, p.purple, 6.0);
                                        ui.label(
                                            RichText::new(format!(
                                                "{} · {} stmt",
                                                crate::widgets::fmt_duration(elapsed),
                                                statements
                                            ))
                                            .size(10.0)
                                            .color(p.purple),
                                        );
                                    } else {
                                        let begin = egui::Button::new(
                                            RichText::new("Begin tx")
                                                .size(10.4)
                                                .color(p.muted),
                                        )
                                        .fill(Color32::TRANSPARENT)
                                        .stroke(Stroke::new(1.0_f32, p.border))
                                        .rounding(Rounding::same(5.0));
                                        if ui
                                            .add_enabled(!any_running, begin)
                                            .on_hover_text(
                                                "BEGIN on the editor connection; later statements join it",
                                            )
                                            .clicked()
                                        {
                                            begin_tx = true;
                                        }
                                    }
                                    let explain = egui::Button::new(
                                        RichText::new("Explain").size(10.5).color(p.muted),
                                    )
                                    .fill(Color32::TRANSPARENT)
                                    .stroke(Stroke::NONE);
                                    if ui
                                        .add_enabled(!any_running, explain)
                                        .on_hover_text("Run EXPLAIN for this statement (⌘E)")
                                        .clicked()
                                    {
                                        explain_requested = true;
                                    }
                                    let fmt = egui::Button::new(
                                        RichText::new("Format").size(10.5).color(p.muted),
                                    )
                                    .fill(Color32::TRANSPARENT)
                                    .stroke(Stroke::NONE);
                                    if ui
                                        .add(fmt)
                                        .on_hover_text("Reformat SQL (⇧⌘F)")
                                        .clicked()
                                    {
                                        format_requested = true;
                                    }
                                    ui.menu_button(
                                        RichText::new("Templates").size(10.5).color(p.muted),
                                        |ui| {
                                            for name in [
                                                "SELECT",
                                                "JOIN",
                                                "INSERT",
                                                "UPDATE",
                                                "DELETE",
                                                "CREATE TABLE",
                                            ] {
                                                if ui.button(name).clicked() {
                                                    template_requested = Some(name);
                                                    ui.close_menu();
                                                }
                                            }
                                        },
                                    );
                                    let hist = egui::Button::new(
                                        RichText::new("History").size(10.5).color(p.muted),
                                    )
                                    .fill(Color32::TRANSPARENT)
                                    .stroke(Stroke::NONE);
                                    if ui.add(hist).clicked() {
                                        open_history = true;
                                    }
                                    egui::ComboBox::from_id_salt(("row-limit", idx))
                                        .selected_text(
                                            RichText::new(crate::app::row_limit_label(
                                                current_limit,
                                            ))
                                            .size(10.5)
                                            .color(p.muted),
                                        )
                                        .width(86.0)
                                        .show_ui(ui, |ui| {
                                            for (value, label) in crate::app::ROW_LIMIT_OPTIONS {
                                                if ui
                                                    .selectable_label(
                                                        current_limit == *value,
                                                        *label,
                                                    )
                                                    .clicked()
                                                {
                                                    chosen_limit = Some(*value);
                                                }
                                            }
                                        });
                                    ui.label(RichText::new("Limit").size(10.5).color(p.faint));
                                    ui.add_space(4.0);
                                });
                            },
                        );
                        let line_rect = ui.available_rect_before_wrap();
                        ui.painter().line_segment(
                            [
                                egui::pos2(line_rect.left(), line_rect.top()),
                                egui::pos2(line_rect.right(), line_rect.top()),
                            ],
                            Stroke::new(1.0_f32, p.border),
                        );
                        ui.add_space(1.0);

                        if let Some(tab) = self.tabs.get_mut(idx) {
                            if let crate::app::TabKind::Query(q) = &mut tab.kind {
                                let line_count = q.sql.lines().count().max(1);
                                ScrollArea::vertical()
                                    .id_salt(("editor", idx))
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        ui.horizontal_top(|ui| {
                                            ui.spacing_mut().item_spacing.x = 0.0;
                                            ui.vertical(|ui| {
                                                ui.spacing_mut().item_spacing.y = 0.0;
                                                ui.add_space(6.0);
                                                for n in 1..=line_count {
                                                    let is_error = q.error_line == Some(n);
                                                    ui.allocate_ui_with_layout(
                                                        Vec2::new(38.0, 16.0),
                                                        Layout::right_to_left(Align::Center),
                                                        |ui| {
                                                            ui.add_space(8.0);
                                                            if is_error {
                                                                ui.painter().rect_filled(
                                                                    Rect::from_min_size(
                                                                        egui::pos2(
                                                                            ui.cursor().left(),
                                                                            ui.cursor().top(),
                                                                        ),
                                                                        Vec2::new(38.0, 16.0),
                                                                    ),
                                                                    Rounding::ZERO,
                                                                    p.red.gamma_multiply(0.18),
                                                                );
                                                            }
                                                            ui.label(
                                                                RichText::new(n.to_string())
                                                                    .monospace()
                                                                    .size(11.0)
                                                                    .color(if is_error {
                                                                        p.red
                                                                    } else {
                                                                        p.faint
                                                                    }),
                                                            );
                                                        },
                                                    );
                                                }
                                            });
                                            ui.add_space(2.0);
                                            let mut layouter =
                                                |ui: &egui::Ui,
                                                 text: &str,
                                                 wrap_width: f32|
                                                 -> std::sync::Arc<egui::Galley> {
                                                    let mut job = widgets::highlight_sql(text, &p);
                                                    job.wrap.max_width = wrap_width;
                                                    ui.fonts(|f| f.layout_job(job))
                                                };
                                            let edit_id =
                                                egui::Id::new(("sql-editor", idx));
                                            let (word_start, word) =
                                                crate::sqlfmt::current_word(&q.sql, q.cursor);
                                            if word != q.completion_word {
                                                q.completion_word = word.clone();
                                                q.completion_index = 0;
                                            }
                                            let focused =
                                                ui.memory(|m| m.has_focus(edit_id));
                                            let suggestions = if focused {
                                                crate::sqlfmt::suggest(
                                                    &word,
                                                    &completion_sources,
                                                    crate::sqlfmt::SQL_KEYWORDS,
                                                    8,
                                                )
                                            } else {
                                                Vec::new()
                                            };
                                            let mut accept: Option<String> = None;
                                            if !suggestions.is_empty() {
                                                let len = suggestions.len();
                                                if ui.input_mut(|i| {
                                                    i.consume_key(
                                                        egui::Modifiers::NONE,
                                                        egui::Key::ArrowDown,
                                                    )
                                                }) {
                                                    q.completion_index =
                                                        (q.completion_index + 1) % len;
                                                } else if ui.input_mut(|i| {
                                                    i.consume_key(
                                                        egui::Modifiers::NONE,
                                                        egui::Key::ArrowUp,
                                                    )
                                                }) {
                                                    q.completion_index = q
                                                        .completion_index
                                                        .checked_sub(1)
                                                        .unwrap_or(len - 1);
                                                } else if ui.input_mut(|i| {
                                                    i.consume_key(
                                                        egui::Modifiers::NONE,
                                                        egui::Key::Tab,
                                                    )
                                                }) {
                                                    accept = suggestions
                                                        .get(q.completion_index.min(len - 1))
                                                        .cloned();
                                                }
                                            }
                                            if let Some(choice) = accept {
                                                let end = q.cursor.min(q.sql.len());
                                                q.sql.replace_range(word_start..end, &choice);
                                                q.cursor = word_start + choice.len();
                                                q.move_cursor = true;
                                                q.dirty = true;
                                            } else if suggestions.is_empty()
                                                && focused
                                                && ui.input_mut(|i| {
                                                    i.consume_key(
                                                        egui::Modifiers::NONE,
                                                        egui::Key::Tab,
                                                    )
                                                })
                                            {
                                                // Indent with the configured tab
                                                // width instead of a hard tab, so
                                                // SQL copied elsewhere keeps the
                                                // same alignment.
                                                let at = q.cursor.min(q.sql.len());
                                                let spaces = " ".repeat(tab_width);
                                                q.sql.insert_str(at, &spaces);
                                                q.cursor = at + tab_width;
                                                q.move_cursor = true;
                                                q.dirty = true;
                                            }
                                            let edit = TextEdit::multiline(&mut q.sql)
                                                .id(edit_id)
                                                .frame(false)
                                                .code_editor()
                                                .desired_width(f32::INFINITY)
                                                .desired_rows(line_count + 2)
                                                .font(TextStyle::Monospace)
                                                .layouter(&mut layouter);
                                            let output = edit.show(ui);
                                            if let Some(range) = output.cursor_range {
                                                q.cursor = range.primary.ccursor.index;
                                            }
                                            if q.move_cursor {
                                                let mut state =
                                                    egui::text_edit::TextEditState::load(
                                                        ui.ctx(),
                                                        edit_id,
                                                    )
                                                    .unwrap_or_default();
                                                state.cursor.set_char_range(Some(
                                                    egui::text::CCursorRange::one(
                                                        egui::text::CCursor::new(q.cursor),
                                                    ),
                                                ));
                                                state.store(ui.ctx(), edit_id);
                                                q.move_cursor = false;
                                            }
                                            if output.response.changed() {
                                                q.dirty = true;
                                            }
                                            if suggestions.is_empty() {
                                                // Nothing to show.
                                            } else if let Some(caret) = output
                                                .cursor_range
                                                .map(|range| {
                                                    output.galley_pos.to_vec2()
                                                        + output
                                                            .galley
                                                            .pos_from_cursor(&range.primary)
                                                            .min
                                                            .to_vec2()
                                                })
                                            {
                                                let mut chosen: Option<String> = None;
                                                egui::Area::new(egui::Id::new((
                                                    "sql-completions",
                                                    idx,
                                                )))
                                                .order(egui::Order::Foreground)
                                                .fixed_pos(egui::pos2(
                                                    caret.x,
                                                    caret.y + 15.0,
                                                ))
                                                .show(ui.ctx(), |ui| {
                                                    egui::Frame::popup(ui.style()).show(
                                                        ui,
                                                        |ui| {
                                                            ui.set_min_width(170.0);
                                                            for (i, item) in
                                                                suggestions.iter().enumerate()
                                                            {
                                                                let selected = i
                                                                    == q.completion_index.min(
                                                                        suggestions.len() - 1,
                                                                    );
                                                                if ui
                                                                    .selectable_label(
                                                                        selected, item,
                                                                    )
                                                                    .clicked()
                                                                {
                                                                    chosen =
                                                                        Some(item.clone());
                                                                }
                                                            }
                                                        },
                                                    );
                                                });
                                                if let Some(choice) = chosen {
                                                    let end = q.cursor.min(q.sql.len());
                                                    q.sql.replace_range(
                                                        word_start..end,
                                                        &choice,
                                                    );
                                                    q.cursor = word_start + choice.len();
                                                    q.move_cursor = true;
                                                    q.dirty = true;
                                                }
                                            }
                                        });
                                    });
                            }
                        }
                    });
            },
        );

        if let Some(limit) = chosen_limit {
            if let Some(tab) = self.tabs.get_mut(idx) {
                if let crate::app::TabKind::Query(q) = &mut tab.kind {
                    q.row_limit = limit;
                }
            }
        }

        // Draggable divider between the editor and the results pane.
        let (handle_rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 9.0), Sense::hover());
        let handle = ui.interact(
            handle_rect,
            egui::Id::new(("query-split", idx)),
            Sense::drag(),
        );
        if handle.hovered() || handle.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        if handle.dragged() {
            let delta = handle.drag_delta().y;
            if let Some(tab) = self.tabs.get_mut(idx) {
                if let crate::app::TabKind::Query(q) = &mut tab.kind {
                    if avail_h > 0.0 {
                        q.split = ((editor_h + delta) / avail_h).clamp(0.15, 0.85);
                    }
                }
            }
        }
        let pill = Rect::from_center_size(handle_rect.center(), Vec2::new(42.0, 2.0));
        ui.painter().rect_filled(
            pill,
            Rounding::same(1.0),
            if handle.hovered() || handle.dragged() {
                p.accent
            } else {
                p.border
            },
        );

        egui::Frame::none()
            .fill(p.panel)
            .stroke(Stroke::new(1.0_f32, p.border))
            .rounding(Rounding::same(8.0))
            .show(ui, |ui| {
                if let Some(tab) = self.tabs.get_mut(idx) {
                    if let crate::app::TabKind::Query(q) = &mut tab.kind {
                        ui.allocate_ui_with_layout(
                            Vec2::new(ui.available_width(), 32.0),
                            Layout::left_to_right(Align::Center),
                            |ui| {
                                ui.add_space(8.0);
                                if widgets::result_tab(
                                    ui,
                                    &p,
                                    "Result",
                                    q.mode == crate::app::ResultMode::Result,
                                )
                                .clicked()
                                {
                                    q.mode = crate::app::ResultMode::Result;
                                }
                                if widgets::result_tab(
                                    ui,
                                    &p,
                                    "Messages",
                                    q.mode == crate::app::ResultMode::Messages,
                                )
                                .clicked()
                                {
                                    q.mode = crate::app::ResultMode::Messages;
                                }
                                if let Some(line) = q.error_line {
                                    ui.add_space(8.0);
                                    let jump = egui::Button::new(
                                        RichText::new(format!("Line {line}"))
                                            .size(9.8)
                                            .color(Color32::WHITE),
                                    )
                                    .fill(p.red)
                                    .rounding(Rounding::same(4.0))
                                    .min_size(Vec2::new(46.0, 18.0));
                                    if ui
                                        .add(jump)
                                        .on_hover_text("Jump to the line reported by the error")
                                        .clicked()
                                    {
                                        jump_to_error = true;
                                    }
                                }
                                if q.mode == crate::app::ResultMode::Result {
                                    ui.add_space(10.0);
                                    let search = ui.add_sized(
                                        Vec2::new(160.0, 22.0),
                                        TextEdit::singleline(&mut q.search)
                                            .id(egui::Id::new(("result-search", idx)))
                                            .hint_text("Search results")
                                            .font(FontId::proportional(11.2)),
                                    );
                                    if focus_search {
                                        search.request_focus();
                                    }
                                    if q.search != q.search_last {
                                        q.search_last = q.search.clone();
                                        q.selected.clear();
                                    }
                                    if !q.search.is_empty() {
                                        let matches = q
                                            .outcome
                                            .as_ref()
                                            .map(|out| {
                                                out.rows
                                                    .iter()
                                                    .filter(|row| {
                                                        row_matches(row, &q.search)
                                                    })
                                                    .count()
                                            })
                                            .unwrap_or(0);
                                        ui.label(
                                            RichText::new(format!("{matches} matched"))
                                                .size(9.8)
                                                .color(p.faint),
                                        );
                                        if widgets::icon_button(ui, &p, "Clear").clicked() {
                                            q.search.clear();
                                            q.search_last.clear();
                                            q.selected.clear();
                                        }
                                    }
                                }
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.add_space(8.0);
                                    if let Some(out) = &q.outcome {
                                        if widgets::copy_button(ui, &p) {
                                            copy_text = Some(widgets::outcome_to_tsv(out));
                                        }
                                        if !q.sort.is_empty() {
                                            let order = q
                                                .sort
                                                .iter()
                                                .map(|spec| {
                                                    format!(
                                                        "{} ({})",
                                                        spec.column,
                                                        if spec.descending { "desc" } else { "asc" }
                                                    )
                                                })
                                                .collect::<Vec<_>>()
                                                .join(", ");
                                            ui.label(
                                                RichText::new(format!("sorted by {order}"))
                                                    .size(9.8)
                                                    .color(p.accent),
                                            );
                                        }
                                        ui.label(
                                            RichText::new(widgets::fmt_ms(out.elapsed_ms))
                                                .size(10.6)
                                                .color(p.faint),
                                        );
                                        ui.label(
                                            RichText::new(format!(
                                                "{} rows · {} cols",
                                                out.rows.len(),
                                                out.columns.len()
                                            ))
                                            .size(10.8)
                                            .color(p.muted),
                                        );
                                    }
                                });
                            },
                        );
                        let sep = ui.available_rect_before_wrap();
                        ui.painter().line_segment(
                            [
                                egui::pos2(sep.left(), sep.top()),
                                egui::pos2(sep.right(), sep.top()),
                            ],
                            Stroke::new(1.0_f32, p.border),
                        );

                        match q.mode {
                            crate::app::ResultMode::Result => {
                                if q.running && q.outcome.is_none() {
                                    widgets::empty_panel(
                                        ui,
                                        &p,
                                        "Running query…",
                                        "The editor stays responsive. Press Cancel to stop it.",
                                    );
                                } else {
                                // Header sorts for query results happen on the
                                // in-memory rows. The flag makes it a one-shot
                                // sort per click instead of every frame. Keep
                                // the original order so clearing the sort
                                // restores the query's own order.
                                if !q.sorted {
                                    if q.sort.is_empty() {
                                        if let Some(out) = q.outcome.as_mut() {
                                            if let Some(original) = &q.original_rows {
                                                out.rows.clone_from(original);
                                            }
                                        }
                                        q.original_rows = None;
                                        q.sorted = true;
                                    } else {
                                        if let Some(out) = q.outcome.as_mut() {
                                            if q.original_rows.is_none() {
                                                q.original_rows = Some(out.rows.clone());
                                            }
                                            if let Some(original) = &q.original_rows {
                                                out.rows.clone_from(original);
                                            }
                                            db::sort_rows(
                                                &mut out.rows,
                                                &q.sort,
                                                &out.columns,
                                            );
                                        }
                                        q.sorted = true;
                                    }
                                }
                                if let Some(out) = &q.outcome {
                                    if let Some(err) = &out.error {
                                        widgets::error_box(ui, &p, err);
                                    } else if out.columns.is_empty() {
                                        widgets::empty_panel(
                                            ui,
                                            &p,
                                            "Statement completed",
                                            &out.ok_message(),
                                        );
                                    } else {
                                        let filtered_rows;
                                        let rows_ref: &[Vec<Option<String>>] =
                                            if q.search.trim().is_empty() {
                                                &out.rows
                                            } else {
                                                filtered_rows =
                                                    filter_query_rows(&out.rows, &q.search);
                                                &filtered_rows
                                            };
                                        match widgets::data_grid(
                                            ui,
                                            &p,
                                            egui::Id::new(("query-grid", idx)),
                                            &out.columns,
                                            rows_ref,
                                            &mut q.selected,
                                            0,
                                            "No rows to show",
                                            None,
                                            false,
                                            &[],
                                            &mut q.sort,
                                            true,
                                            grid_style,
                                            &mut q.widths,
                                            &[],
                                            &mut q.scroll,
                                            q.restore_scroll.take(),
                                        ) {
                                            widgets::GridAction::CopyText(text) => {
                                                copy_text = Some(text);
                                            }
                                            widgets::GridAction::SortChanged => {
                                                q.sorted = false;
                                                q.selected.clear();
                                            }
                                            _ => {}
                                        }
                                    }
                                } else {
                                    widgets::empty_panel(
                                        ui,
                                        &p,
                                        "Run a query to see results",
                                        "Write SQL above, then press Cmd+Enter.",
                                    );
                                }
                                }
                            }
                            crate::app::ResultMode::Messages => {
                                widgets::messages_ui(ui, &p, q.outcome.as_ref());
                            }
                        }
                    }
                }
            });

        if let Some(text) = copy_text {
            self.pending_copy = Some(text);
        }
        if open_history {
            self.history_open = true;
        }
        if cancel_requested {
            self.cancel_query();
        }
        if run_requested {
            self.run_active();
        }
        if format_requested {
            self.format_active_sql();
        }
        if begin_tx {
            self.begin_transaction();
        }
        if commit_tx {
            self.commit_transaction();
        }
        if rollback_tx {
            self.rollback_transaction();
        }
        if explain_requested {
            self.explain_active();
        }
        if let Some(name) = template_requested {
            self.insert_sql_template(name);
        }
        if jump_to_error {
            if let Some(tab) = self.tabs.get_mut(idx) {
                if let crate::app::TabKind::Query(q) = &mut tab.kind {
                    if let Some(line) = q.error_line {
                        q.cursor = offset_of_line(&q.sql, line);
                        q.move_cursor = true;
                    }
                }
            }
            ui.ctx()
                .memory_mut(|memory| memory.request_focus(egui::Id::new(("sql-editor", idx))));
        }
        self.focus_search = false;
    }

    pub(crate) fn history_window(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let mut open = self.history_open;
        let mut load: Option<String> = None;

        egui::Window::new("Query history")
            .open(&mut open)
            .collapsible(false)
            .default_width(520.0)
            .max_height(430.0)
            .anchor(Align2::RIGHT_TOP, Vec2::new(-18.0, 58.0))
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(Rounding::same(10.0))
                    .inner_margin(Margin::symmetric(14.0, 12.0)),
            )
            .show(ctx, |ui| {
                ui.label(RichText::new("This session").size(10.8).color(p.faint));
                ui.add_space(6.0);
                if self.query_history.is_empty() {
                    ui.label(
                        RichText::new("Queries you run will appear here.")
                            .size(11.6)
                            .color(p.muted),
                    );
                    return;
                }
                ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    for (i, sql) in self.query_history.iter().rev().enumerate() {
                        let preview = sql
                            .lines()
                            .map(str::trim)
                            .find(|line| !line.is_empty() && !line.starts_with("--"))
                            .unwrap_or(sql)
                            .to_string();
                        egui::Frame::none()
                            .fill(p.card)
                            .stroke(Stroke::new(1.0_f32, p.border))
                            .rounding(Rounding::same(6.0))
                            .inner_margin(Margin::symmetric(10.0, 7.0))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(format!("{:02}", i + 1))
                                            .monospace()
                                            .size(9.8)
                                            .color(p.faint),
                                    );
                                    ui.label(
                                        RichText::new(widgets::truncate(&preview, 58))
                                            .monospace()
                                            .size(11.2)
                                            .color(p.text),
                                    );
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if ui
                                            .add(
                                                egui::Button::new(
                                                    RichText::new("Use").size(10.5).color(p.accent),
                                                )
                                                .fill(Color32::TRANSPARENT)
                                                .stroke(Stroke::NONE),
                                            )
                                            .clicked()
                                        {
                                            load = Some(sql.clone());
                                        }
                                    });
                                });
                            });
                        ui.add_space(5.0);
                    }
                });
            });

        self.history_open = open;
        if let Some(sql) = load {
            if let Some(tab) = self.tabs.get_mut(self.active) {
                if let crate::app::TabKind::Query(q) = &mut tab.kind {
                    q.sql = sql;
                    q.outcome = None;
                    q.selected.clear();
                    q.sort.clear();
                    q.sorted = true;
                    q.original_rows = None;
                }
            }
            self.history_open = false;
        }
    }

    pub(crate) fn table_tab_ui(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let idx = self.active;
        let mut refresh = false;
        let mut apply_filter = false;
        let mut commit_cell: Option<CellCommit> = None;
        let mut copy_cell: Option<String> = None;
        let mut export_request: Option<crate::app::ExportFormat> = None;
        let mut open_add_row = false;
        let mut open_delete_row: Option<(db::TableId, String, String, usize)> = None;
        let mut open_add_column = false;
        let mut edit_column: Option<String> = None;
        let mut delete_column: Option<String> = None;
        let mut expand_value: Option<(usize, usize)> = None;
        let read_only = self.is_read_only();
        let grid_style = widgets::GridStyle {
            row_height: self.settings.row_height,
            font_size: self.settings.data_font_size,
            alternate_rows: self.settings.alternate_rows,
            show_row_numbers: self.settings.show_row_numbers,
        };

        if let Some(tab) = self.tabs.get_mut(idx) {
            if let crate::app::TabKind::Table(tt) = &mut tab.kind {
                egui::Frame::none()
                    .fill(p.panel)
                    .inner_margin(Margin::symmetric(12.0, 7.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            let (icon_rect, _) =
                                ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                            widgets::draw_table_icon(ui.painter(), icon_rect, p.accent);
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new(tt.table.display())
                                    .size(14.0)
                                    .strong()
                                    .color(p.text),
                            );
                            ui.add_space(4.0);
                            widgets::chip(ui, &p, "table", p.purple);
                            ui.add_space(12.0);

                            if widgets::segmented(ui, &p, "Data", tt.mode == TableMode::Data)
                                .clicked()
                            {
                                tt.mode = TableMode::Data;
                            }
                            if widgets::segmented(
                                ui,
                                &p,
                                "Structure",
                                tt.mode == TableMode::Structure,
                            )
                            .clicked()
                            {
                                tt.mode = TableMode::Structure;
                            }

                            if tt.mode == TableMode::Data {
                                ui.add_space(4.0);
                                // With the row editor open the centre pane is
                                // narrower, so labels and the quick filter
                                // shrink instead of clipping.
                                let compact = tt.detail_open;
                                let add_label = if compact {
                                    "+ Record"
                                } else {
                                    "+ Add record"
                                };
                                if ui
                                    .add_enabled_ui(!read_only, |ui| {
                                        widgets::icon_button(ui, &p, add_label)
                                            .on_hover_text("Add a new record")
                                            .clicked()
                                    })
                                    .inner
                                {
                                    open_add_row = true;
                                }
                                let delete_target = tt.selected.cursor_row().and_then(|row| {
                                    let key_value = tt.data.rowids.get(row).cloned().flatten()?;
                                    let key_column = tt.data.key_column.clone()?;
                                    Some((row, key_column, key_value))
                                });
                                let delete_label = if compact {
                                    "Delete"
                                } else {
                                    "Delete record"
                                };
                                let delete_button =
                                    egui::Button::new(RichText::new(delete_label).size(11.0))
                                        .fill(p.card)
                                        .stroke(Stroke::new(1.0_f32, p.border));
                                if ui
                                    .add_enabled(
                                        delete_target.is_some() && !read_only,
                                        delete_button,
                                    )
                                    .on_hover_text("Delete the row at the cursor")
                                    .clicked()
                                {
                                    if let Some((row, key_column, key_value)) = delete_target {
                                        open_delete_row = Some((
                                            tt.table.clone(),
                                            key_column,
                                            key_value,
                                            tt.offset + row + 1,
                                        ));
                                    }
                                }
                                // While open, the header offers a compact hide
                                // button; the always-visible "Row detail"
                                // button in the main toolbar brings it back.
                                if tt.detail_open {
                                    ui.add_space(2.0);
                                    if widgets::row_detail_button(ui, &p, true)
                                        .on_hover_text(
                                            "Hide the vertical row editor (Space)",
                                        )
                                        .clicked()
                                    {
                                        tt.detail_open = false;
                                    }
                                }
                            }

                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if widgets::icon_button(ui, &p, "Refresh").clicked() {
                                    refresh = true;
                                }
                                if tt.mode == TableMode::Data {
                                    let filter_active = !tt.applied_filter.is_empty();
                                    let export = ui.menu_button(
                                        RichText::new("Export").size(11.0).color(p.text),
                                        |ui| {
                                            ui.set_min_width(210.0);
                                            ui.label(
                                                RichText::new(if filter_active {
                                                    "Rows matching the current filter"
                                                } else {
                                                    "Every row in the table"
                                                })
                                                .size(10.0)
                                                .color(p.faint),
                                            );
                                            ui.separator();
                                            if ui.button("CSV…").clicked() {
                                                export_request =
                                                    Some(crate::app::ExportFormat::Csv);
                                                ui.close_menu();
                                            }
                                            if ui.button("Excel workbook…").clicked() {
                                                export_request =
                                                    Some(crate::app::ExportFormat::Xlsx);
                                                ui.close_menu();
                                            }
                                            if ui.button("SQL dump…").clicked() {
                                                export_request =
                                                    Some(crate::app::ExportFormat::Sql);
                                                ui.close_menu();
                                            }
                                        },
                                    );
                                    let _ = export.response.on_hover_text(if filter_active {
                                        "Export every row matching the filter, not just this page"
                                    } else {
                                        "Export every row in the table"
                                    });
                                }
                                if tt.mode == TableMode::Data {
                                    let filter_width = if tt.detail_open { 140.0 } else { 200.0 };
                                    let resp = ui.add_sized(
                                        Vec2::new(filter_width, 25.0),
                                        TextEdit::singleline(&mut tt.filter.quick)
                                            .hint_text("Quick Filter"),
                                    );
                                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                                    if enter && (resp.has_focus() || resp.lost_focus()) {
                                        apply_filter = true;
                                    }
                                    // Column filters live behind this toggle so
                                    // the bar stays compact when unused.
                                    let active = tt.filter.active_count();
                                    let label = if active == 0 {
                                        "Filter".to_string()
                                    } else {
                                        format!("Filter ({active})")
                                    };
                                    if widgets::icon_button(ui, &p, &label)
                                        .on_hover_text("Filter by one or more columns")
                                        .clicked()
                                    {
                                        tt.filters_open = !tt.filters_open;
                                    }
                                }
                            });
                        });

                        if tt.mode == TableMode::Data && tt.filters_open {
                            let names: Vec<String> =
                                tt.structure.iter().map(|c| c.name.clone()).collect();
                            ui.add_space(7.0);
                            let mut remove_row: Option<usize> = None;
                            for i in 0..tt.filter.columns.len() {
                                ui.horizontal(|ui| {
                                    let filter = &mut tt.filter.columns[i];
                                    egui::ComboBox::from_id_salt(("filter-column", idx, i))
                                        .selected_text(
                                            RichText::new(if filter.column.is_empty() {
                                                "Column…"
                                            } else {
                                                filter.column.as_str()
                                            })
                                            .size(11.0)
                                            .color(p.text),
                                        )
                                        .width(150.0)
                                        .show_ui(ui, |ui| {
                                            for name in &names {
                                                if ui
                                                    .selectable_value(
                                                        &mut filter.column,
                                                        name.clone(),
                                                        name,
                                                    )
                                                    .clicked()
                                                {
                                                    apply_filter = true;
                                                }
                                            }
                                        });
                                    egui::ComboBox::from_id_salt(("filter-op", idx, i))
                                        .selected_text(
                                            RichText::new(filter.op.label())
                                                .size(11.0)
                                                .color(p.text),
                                        )
                                        .width(126.0)
                                        .show_ui(ui, |ui| {
                                            for op in db::FilterOp::ALL {
                                                if ui
                                                    .selectable_value(
                                                        &mut filter.op,
                                                        op,
                                                        op.label(),
                                                    )
                                                    .clicked()
                                                {
                                                    apply_filter = true;
                                                }
                                            }
                                        });
                                    if filter.op.needs_value() {
                                        let resp = ui.add_sized(
                                            Vec2::new(170.0, 22.0),
                                            TextEdit::singleline(&mut filter.value)
                                                .hint_text("value"),
                                        );
                                        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                                        if resp.lost_focus() || (enter && resp.has_focus()) {
                                            apply_filter = true;
                                        }
                                    } else {
                                        ui.label(RichText::new("—").size(11.0).color(p.faint));
                                    }
                                    if widgets::icon_button(ui, &p, "Remove").clicked() {
                                        remove_row = Some(i);
                                    }
                                });
                            }
                            ui.horizontal(|ui| {
                                if widgets::icon_button(ui, &p, "+ Add filter").clicked() {
                                    let column = names.first().cloned().unwrap_or_default();
                                    tt.filter.columns.push(db::ColumnFilter::new(column));
                                }
                                if tt.filter.columns.len() > 1 {
                                    ui.add_space(8.0);
                                    ui.label(RichText::new("match").size(10.4).color(p.faint));
                                    if widgets::segmented(ui, &p, "all", !tt.filter.any)
                                        .on_hover_text("Every filter must match (AND)")
                                        .clicked()
                                        && tt.filter.any
                                    {
                                        tt.filter.any = false;
                                        apply_filter = true;
                                    }
                                    if widgets::segmented(ui, &p, "any", tt.filter.any)
                                        .on_hover_text("At least one filter must match (OR)")
                                        .clicked()
                                        && !tt.filter.any
                                    {
                                        tt.filter.any = true;
                                        apply_filter = true;
                                    }
                                }
                                if !tt.filter.is_empty() {
                                    ui.add_space(8.0);
                                    if widgets::icon_button(ui, &p, "Clear filters").clicked() {
                                        tt.filter = db::TableFilter::default();
                                        apply_filter = true;
                                    }
                                }
                            });
                            if let Some(i) = remove_row {
                                tt.filter.columns.remove(i);
                                apply_filter = true;
                            }
                        }
                    });

                let line = ui.available_rect_before_wrap();
                ui.painter().line_segment(
                    [
                        egui::pos2(line.left(), line.top()),
                        egui::pos2(line.right(), line.top()),
                    ],
                    Stroke::new(1.0_f32, p.border),
                );

                match tt.mode {
                    TableMode::Data => {
                        if let Some(err) = &tt.data.error {
                            widgets::error_box(ui, &p, err);
                        } else {
                            let grid_height = (ui.available_height() - 34.0).max(100.0);
                            let null_allowed: Vec<bool> = tt
                                .data
                                .columns
                                .iter()
                                .map(|name| {
                                    tt.structure
                                        .iter()
                                        .find(|column| &column.name == name)
                                        .map(|column| !column.not_null && !column.primary_key)
                                        .unwrap_or(true)
                                })
                                .collect();
                            let dbl = ui
                                .allocate_ui_with_layout(
                                    Vec2::new(ui.available_width(), grid_height),
                                    Layout::top_down(Align::Min),
                                    |ui| {
                                        let column_types: Vec<String> = tt
                                            .data
                                            .columns
                                            .iter()
                                            .map(|name| {
                                                tt.structure
                                                    .iter()
                                                    .find(|column| &column.name == name)
                                                    .map(|column| column.decl_type.clone())
                                                    .unwrap_or_default()
                                            })
                                            .collect();
                                        widgets::data_grid(
                                            ui,
                                            &p,
                                            egui::Id::new(("table-grid", idx)),
                                            &tt.data.columns,
                                            &tt.data.rows,
                                            &mut tt.selected,
                                            tt.offset,
                                            if tt.applied_filter.is_empty() {
                                                "No rows to show"
                                            } else {
                                                // `sub_10000e300` (0x10000e300).
                                                "No matching columns or rows"
                                            },
                                            tt.edit.as_mut(),
                                            tt.data.editable && !read_only,
                                            &null_allowed,
                                            &mut tt.sort,
                                            true,
                                            grid_style,
                                            &mut tt.widths,
                                            &column_types,
                                            &mut tt.scroll,
                                            tt.restore_scroll.take(),
                                        )
                                    },
                                )
                                .inner;
                            match dbl {
                                widgets::GridAction::OpenEditor(row, col) => {
                                    // Views and keyless tables cannot be edited.
                                    let editable = tt.data.key_column.is_some()
                                        && tt.data.rowids.get(row).cloned().flatten().is_some();
                                    if editable && col < tt.data.columns.len() {
                                        let original = tt
                                            .data
                                            .rows
                                            .get(row)
                                            .and_then(|values| values.get(col))
                                            .cloned()
                                            .flatten();
                                        let text = original.clone().unwrap_or_default();
                                        let kind = column_editor_kind(tt, col);
                                        match &kind {
                                            // Double-clicking a boolean flips
                                            // it, like a checkbox.
                                            crate::db::FieldEditor::Bool => {
                                                let current = text.trim();
                                                let value = if widgets::is_truthy(current) {
                                                    "0"
                                                } else {
                                                    "1"
                                                };
                                                if let Some(commit) = cell_commit(
                                                    tt,
                                                    idx,
                                                    row,
                                                    col,
                                                    Some(value.to_string()),
                                                ) {
                                                    commit_cell = Some(commit);
                                                }
                                            }
                                            // JSON and binary values do not fit
                                            // a 22 px cell.
                                            kind if kind.is_modal() => {
                                                expand_value = Some((row, col));
                                            }
                                            _ => {
                                                tt.edit = Some(widgets::GridEdit {
                                                    row,
                                                    col,
                                                    text,
                                                    original,
                                                    focus: true,
                                                    kind,
                                                });
                                            }
                                        }
                                    }
                                }
                                widgets::GridAction::Commit => {
                                    if let Some(edit) = tt.edit.take() {
                                        // An untouched NULL stays NULL; clearing
                                        // a non-NULL value writes an empty
                                        // string, matching the row detail menu.
                                        let value =
                                            edited_cell_value(&edit.original, &edit.text);
                                        if let Some(commit) =
                                            cell_commit(tt, idx, edit.row, edit.col, value)
                                        {
                                            commit_cell = Some(commit);
                                        }
                                    }
                                }
                                widgets::GridAction::Cancel => {
                                    tt.edit = None;
                                }
                                widgets::GridAction::SetNull(row, col) => {
                                    if let Some(commit) =
                                        cell_commit(tt, idx, row, col, None)
                                    {
                                        commit_cell = Some(commit);
                                    }
                                }
                                widgets::GridAction::SetEmpty(row, col) => {
                                    if let Some(commit) = cell_commit(
                                        tt,
                                        idx,
                                        row,
                                        col,
                                        Some(String::new()),
                                    ) {
                                        commit_cell = Some(commit);
                                    }
                                }
                                widgets::GridAction::CopyText(text) => {
                                    copy_cell = Some(text);
                                }
                                widgets::GridAction::SortChanged => {
                                    // The next fetch applies the new ORDER BY;
                                    // jump to the first page so the new order is
                                    // visible instead of an arbitrary page.
                                    tt.selected.clear();
                                    tt.offset = 0;
                                    refresh = true;
                                }
                                widgets::GridAction::SetValue(row, col, value) => {
                                    tt.edit = None;
                                    if let Some(commit) =
                                        cell_commit(tt, idx, row, col, value)
                                    {
                                        commit_cell = Some(commit);
                                    }
                                }
                                widgets::GridAction::ExpandValue(row, col) => {
                                    expand_value = Some((row, col));
                                }
                                widgets::GridAction::WidthsChanged => {}
                                widgets::GridAction::None => {}
                            }

                            ui.allocate_ui_with_layout(
                                Vec2::new(ui.available_width(), 30.0),
                                Layout::left_to_right(Align::Center),
                                |ui| {
                                    ui.add_space(12.0);
                                    let loaded = tt.data.rows.len();
                                    let total = tt.data.total.max(0) as usize;
                                    let truncated =
                                        tt.limit > 0 && loaded == tt.limit && total > loaded;
                                    let status = if truncated {
                                        // `sub_100018d60` (0x100018d60).
                                        format!("Only {loaded} rows loaded")
                                    } else if total > 0 {
                                        // `sub_10000e300` (0x10000e300):
                                        // "Rows (%@ of %@ loaded)".
                                        format!("Rows ({loaded} of {total} loaded)")
                                    } else {
                                        // `sub_10000e300`: "Rows (%@ so far)"
                                        // while the total is unknown.
                                        format!("Rows ({loaded} so far)")
                                    };
                                    ui.label(RichText::new(status).size(11.0).color(p.muted));
                                    ui.add_space(8.0);
                                    if tt.sort.is_empty() {
                                        ui.label(
                                            RichText::new("Click a column header to sort")
                                                .size(10.3)
                                                .color(p.faint),
                                        );
                                    } else {
                                        let order = tt
                                            .sort
                                            .iter()
                                            .map(|spec| {
                                                format!(
                                                    "{} ({})",
                                                    spec.column,
                                                    if spec.descending { "desc" } else { "asc" }
                                                )
                                            })
                                            .collect::<Vec<_>>()
                                            .join(", ");
                                        ui.label(
                                            RichText::new(format!("Sorted by {order}"))
                                                .size(10.3)
                                                .color(p.accent),
                                        );
                                        if widgets::icon_button(ui, &p, "Clear sort").clicked() {
                                            tt.sort.clear();
                                            tt.offset = 0;
                                            refresh = true;
                                        }
                                    }
                                    if let Some((selected_rows, selected_cols)) =
                                        tt.selected.summary()
                                    {
                                        if selected_rows * selected_cols > 1 {
                                            ui.add_space(8.0);
                                            ui.label(
                                                RichText::new(format!(
                                                    "{selected_rows}×{selected_cols} selected"
                                                ))
                                                .size(10.6)
                                                .color(p.accent),
                                            );
                                        }
                                    }
                                    ui.add_space(10.0);
                                    ui.label(
                                        RichText::new(widgets::fmt_ms(tt.data.elapsed_ms))
                                            .size(10.6)
                                            .color(p.faint),
                                    );
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.add_space(10.0);
                                        let step = db::page_step(tt.limit);
                                        let has_next = (tt.offset + step) < total;
                                        if ui
                                            .add_enabled(
                                                has_next,
                                                egui::Button::new(RichText::new("Next").size(11.0))
                                                    .fill(p.card)
                                                    .stroke(Stroke::new(1.0_f32, p.border)),
                                            )
                                            .on_hover_text("Next page (Cmd+Right)")
                                            .clicked()
                                        {
                                            tt.offset += step;
                                            refresh = true;
                                        }
                                        if ui
                                            .add_enabled(
                                                tt.offset > 0,
                                                egui::Button::new(
                                                    RichText::new("Previous").size(11.0),
                                                )
                                                .fill(p.card)
                                                .stroke(Stroke::new(1.0_f32, p.border)),
                                            )
                                            .on_hover_text("Previous page (Cmd+Left)")
                                            .clicked()
                                        {
                                            tt.offset = tt.offset.saturating_sub(step);
                                            refresh = true;
                                        }
                                        if ui
                                            .add(
                                                egui::Button::new(
                                                    RichText::new("Page settings").size(11.0),
                                                )
                                                .fill(p.card)
                                                .stroke(Stroke::new(1.0_f32, p.border)),
                                            )
                                            .clicked()
                                        {
                                            tt.paging_open = true;
                                            // `sub_10081f074` (0x10081f074)
                                            // shows `max(limit, 300)` in the
                                            // Limit field, not the raw stored
                                            // limit.
                                            tt.limit_text = db::page_step(tt.limit).to_string();
                                            tt.offset_text = tt.offset.to_string();
                                            tt.paging_error = None;
                                        }
                                        ui.label(
                                            RichText::new(format!(
                                                "Limit {}",
                                                if tt.limit == 0 {
                                                    "No limit".to_string()
                                                } else {
                                                    tt.limit.to_string()
                                                }
                                            ))
                                            .size(10.6)
                                            .color(p.faint),
                                        );
                                    });
                                },
                            );
                        }
                    }
                    TableMode::Structure => {
                        let available = ui.available_height();
                        ScrollArea::vertical()
                            .id_salt(("structure", idx))
                            .auto_shrink([false, false])
                            .max_height(available)
                            .show(ui, |ui| {
                                ui.add_space(10.0);
                                ui.horizontal(|ui| {
                                    ui.add_space(14.0);
                                    ui.label(
                                        RichText::new("COLUMNS").size(10.4).strong().color(p.faint),
                                    );
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        ui.add_space(14.0);
                                        if ui
                                            .add_enabled_ui(!read_only, |ui| {
                                                widgets::icon_button(ui, &p, "+ Add column")
                                                    .clicked()
                                            })
                                            .inner
                                        {
                                            open_add_column = true;
                                        }
                                    });
                                });
                                ui.add_space(8.0);
                                egui::Frame::none()
                                    .fill(p.panel)
                                    .stroke(Stroke::new(1.0_f32, p.border))
                                    .rounding(Rounding::same(8.0))
                                    .inner_margin(Margin::symmetric(14.0, 10.0))
                                    .show(ui, |ui| {
                                        ui.set_max_width((ui.available_width() - 4.0).max(240.0));
                                        egui::Grid::new(("cols", idx))
                                            .num_columns(6)
                                            .striped(true)
                                            .spacing([18.0, 9.0])
                                            .min_col_width(70.0)
                                            .show(ui, |ui| {
                                                for h in [
                                                    "Name", "Type", "Nullable", "Default",
                                                    "Primary", "",
                                                ] {
                                                    ui.label(
                                                        RichText::new(h)
                                                            .size(10.8)
                                                            .strong()
                                                            .color(p.muted),
                                                    );
                                                }
                                                ui.end_row();
                                                for c in &tt.structure {
                                                    ui.label(
                                                        RichText::new(&c.name)
                                                            .monospace()
                                                            .size(12.0)
                                                            .color(p.text),
                                                    );
                                                    ui.label(
                                                        RichText::new(if c.decl_type.is_empty() {
                                                            "—"
                                                        } else {
                                                            &c.decl_type
                                                        })
                                                        .size(11.8)
                                                        .color(p.muted),
                                                    );
                                                    ui.label(
                                                        RichText::new(
                                                            if c.not_null || c.primary_key {
                                                                "NO"
                                                            } else {
                                                                "YES"
                                                            },
                                                        )
                                                        .size(11.8)
                                                        .color(if c.not_null || c.primary_key {
                                                            p.text
                                                        } else {
                                                            p.faint
                                                        }),
                                                    );
                                                    ui.label(
                                                        RichText::new(
                                                            c.default_value
                                                                .as_deref()
                                                                .unwrap_or("—"),
                                                        )
                                                        .size(11.8)
                                                        .color(p.faint),
                                                    );
                                                    ui.label(
                                                        RichText::new(if c.primary_key {
                                                            "PK"
                                                        } else {
                                                            ""
                                                        })
                                                        .size(11.8)
                                                        .color(p.amber),
                                                    );
                                                    ui.horizontal(|ui| {
                                                        if ui
                                                            .add_enabled_ui(!read_only, |ui| {
                                                                widgets::icon_button(ui, &p, "Edit")
                                                                    .clicked()
                                                            })
                                                            .inner
                                                        {
                                                            edit_column = Some(c.name.clone());
                                                        }
                                                        if ui
                                                            .add_enabled_ui(!read_only, |ui| {
                                                                widgets::icon_button(ui, &p, "Delete")
                                                                    .clicked()
                                                            })
                                                            .inner
                                                        {
                                                            delete_column = Some(c.name.clone());
                                                        }
                                                    });
                                                    ui.end_row();
                                                }
                                            });
                                    });

                                ui.add_space(18.0);
                                ui.horizontal(|ui| {
                                    ui.add_space(14.0);
                                    ui.label(
                                        RichText::new("INDEXES").size(10.4).strong().color(p.faint),
                                    );
                                });
                                ui.add_space(6.0);
                                if tt.indexes.is_empty() {
                                    ui.horizontal(|ui| {
                                        ui.add_space(14.0);
                                        ui.label(
                                            RichText::new("No indexes").size(11.2).color(p.faint),
                                        );
                                    });
                                } else {
                                    for index in &tt.indexes {
                                        ui.horizontal(|ui| {
                                            ui.add_space(14.0);
                                            ui.label(
                                                RichText::new(&index.name)
                                                    .monospace()
                                                    .size(11.6)
                                                    .color(p.text),
                                            );
                                            ui.label(
                                                RichText::new(if index.unique {
                                                    "unique"
                                                } else {
                                                    ""
                                                })
                                                .size(10.5)
                                                .color(p.amber),
                                            );
                                            ui.label(
                                                RichText::new(index.columns.join(", "))
                                                    .size(11.0)
                                                    .color(p.faint),
                                            );
                                        });
                                    }
                                }

                                ui.add_space(18.0);
                                ui.horizontal(|ui| {
                                    ui.add_space(14.0);
                                    ui.label(
                                        RichText::new("DDL").size(10.4).strong().color(p.faint),
                                    );
                                });
                                ui.add_space(6.0);
                                ui.horizontal(|ui| {
                                    ui.add_space(14.0);
                                    egui::Frame::none()
                                        .fill(p.editor)
                                        .stroke(Stroke::new(1.0_f32, p.border))
                                        .rounding(Rounding::same(6.0))
                                        .inner_margin(Margin::symmetric(12.0, 10.0))
                                        .show(ui, |ui| {
                                            ui.set_max_width(
                                                (ui.available_width() - 18.0).max(200.0),
                                            );
                                            ui.label(
                                                RichText::new(
                                                    tt.ddl.as_deref().unwrap_or("No DDL available"),
                                                )
                                                .monospace()
                                                .size(11.4)
                                                .color(p.code),
                                            );
                                        });
                                });
                                ui.add_space(18.0);
                            });
                    }
                }
            }
        }

        if apply_filter {
            if let Some(tab) = self.tabs.get_mut(idx) {
                if let crate::app::TabKind::Table(tt) = &mut tab.kind {
                    tt.applied_filter = tt.filter.clone();
                    tt.offset = 0;
                    refresh = true;
                }
            }
        }
        if refresh {
            self.refresh_table_tab(idx);
        }
        if let Some((row, col)) = expand_value {
            self.open_value_editor(idx, row, col);
        }
        if let Some(text) = copy_cell {
            self.pending_copy = Some(text);
        }
        if let Some(commit) = commit_cell {
            let table = commit.table.sql();
            let result = match &self.live {
                Some(live) => live.db.update_cell(
                    &table,
                    &commit.column,
                    &commit.key_column,
                    &commit.key_value,
                    commit.value.as_deref(),
                ),
                None => Err("The connection is closed.".to_string()),
            };
            match result {
                Ok(affected) => {
                    self.record_edit(
                        &commit.table,
                        &commit.key_column,
                        &commit.key_value,
                        &commit.column,
                        commit.original.clone(),
                        commit.value.clone(),
                    );
                    self.refresh_table_tab(commit.tab);
                    let driver = self
                        .live
                        .as_ref()
                        .map(|live| live.db.driver)
                        .unwrap_or(Driver::Sqlite);
                    if affected == 0 && driver != Driver::MySql {
                        self.push_toast(
                            "No row was updated; it may have been deleted.".to_string(),
                            ToastKind::Error,
                        );
                    } else {
                        self.push_toast(
                            format!(
                                "Updated {} row{}",
                                affected,
                                if affected == 1 { "" } else { "s" }
                            ),
                            ToastKind::Info,
                        );
                    }
                }
                Err(error) => {
                    self.push_toast(error, ToastKind::Error);
                    // Keep the editor (and the typed text) open on failure.
                    if let Some(tab) = self.tabs.get_mut(commit.tab) {
                        if let crate::app::TabKind::Table(tt) = &mut tab.kind {
                            if commit.row < tt.data.rows.len() && commit.col < tt.data.columns.len()
                            {
                                let kind = column_editor_kind(tt, commit.col);
                                tt.edit = Some(widgets::GridEdit {
                                    row: commit.row,
                                    col: commit.col,
                                    text: commit.value.unwrap_or_default(),
                                    original: commit.original,
                                    focus: true,
                                    kind,
                                });
                            }
                        }
                    }
                }
            }
        }
        // Runs after any pending cell write so the file reflects it.
        if let Some(format) = export_request {
            self.export_active_table_view(format);
        }
        if open_add_row {
            self.open_row_editor(idx);
        }
        if open_add_column {
            self.open_column_editor(idx, None);
        }
        if let Some(column) = edit_column {
            self.open_column_editor(idx, Some(column));
        }
        if let Some(column) = delete_column {
            self.confirm_delete_column(idx, column);
        }
        if let Some((table, key_column, key_value, row_number)) = open_delete_row {
            self.confirm_delete_row(idx, table, key_column, key_value, row_number);
        }
        // Keep the vertical row editor in step with the grid selection.
        self.sync_row_detail(idx);
        self.focus_search = false;
        let ctx = ui.ctx().clone();
        self.paging_window(&ctx);
    }

    /// Rebuilds the row detail form when the grid selection changes or the
    /// loaded page changes underneath it.
    pub(crate) fn sync_row_detail(&mut self, idx: usize) {
        let driver = self
            .live
            .as_ref()
            .map(|live| live.db.driver)
            .unwrap_or(Driver::Sqlite);
        let Some(tab) = self.tabs.get_mut(idx) else {
            return;
        };
        let crate::app::TabKind::Table(tt) = &mut tab.kind else {
            return;
        };
        let row = tt.selected.cursor_row();
        match row {
            Some(row) if row < tt.data.rows.len() => {
                let stale = tt
                    .detail
                    .as_ref()
                    .map(|detail| detail.row != row)
                    .unwrap_or(true);
                if stale {
                    tt.detail = tt.build_row_detail(row, driver);
                }
            }
            _ => tt.detail = None,
        }
    }

    /// The right-hand vertical row editor from TablePlus: one searchable field
    /// per row, editable in place, saved when the field loses focus (Enter in
    /// a single-line field, Cmd+Enter in a multi-line one). Mirrors
    /// `RightSidebarController` / `RowDetailTable` in the original.
    pub(crate) fn row_detail_panel(&mut self, ctx: &egui::Context) {
        let idx = self.active;
        let p = self.palette;
        let connected = self.live.is_some();
        let read_only = self.is_read_only();
        let mut commits: Vec<(String, Option<String>)> = Vec::new();
        let mut copy: Option<String> = None;

        {
            let Some(tab) = self.tabs.get_mut(idx) else {
                return;
            };
            let crate::app::TabKind::Table(tt) = &mut tab.kind else {
                return;
            };
            if !connected || tt.mode != TableMode::Data || !tt.detail_open {
                return;
            }

            let editable = tt.data.editable
                && !read_only
                && tt
                    .detail
                    .as_ref()
                    .map(|detail| !detail.key_column.is_empty())
                    .unwrap_or(false);
            let mut hide_panel = false;

            egui::SidePanel::right("row_detail")
                .resizable(true)
                .default_width(crate::app::ROW_DETAIL_W + 36.0)
                .min_width(crate::app::ROW_DETAIL_MIN_W)
                .max_width(crate::app::ROW_DETAIL_W + 260.0)
                .frame(
                    egui::Frame::none()
                        .fill(p.panel)
                        .stroke(Stroke::new(1.0_f32, p.border))
                        .inner_margin(Margin {
                            left: 14.0,
                            right: 14.0,
                            top: 9.0,
                            bottom: 8.0,
                        }),
                )
                .show(ctx, |ui| {
                    ui.add_space(2.0);
                    // Search field: magnifier + "Search for field" + clear
                    // chevron, with the matching-strategy menu beside it.
                    let search_h = 32.0;
                    let (row_rect, _) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), search_h),
                        Sense::hover(),
                    );
                    let search_rect = Rect::from_min_size(
                        row_rect.min,
                        Vec2::new((row_rect.width() - 30.0).max(80.0), search_h),
                    );
                    let options_rect = Rect::from_min_size(
                        egui::pos2(row_rect.right() - 24.0, row_rect.top() + 2.0),
                        Vec2::splat(24.0),
                    );
                    ui.painter()
                        .rect_filled(search_rect, Rounding::same(8.0), p.editor);
                    ui.painter().rect_stroke(
                        search_rect,
                        Rounding::same(8.0),
                        Stroke::new(1.0_f32, p.border),
                    );
                    let icon_rect = Rect::from_center_size(
                        egui::pos2(search_rect.left() + 16.0, search_rect.center().y),
                        Vec2::splat(13.0),
                    );
                    widgets::draw_search_icon(ui.painter(), icon_rect, p.faint);
                    let text_rect = Rect::from_min_max(
                        egui::pos2(search_rect.left() + 30.0, search_rect.top() + 5.0),
                        egui::pos2(search_rect.right() - 26.0, search_rect.bottom() - 5.0),
                    );
                    let mut search_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(text_rect)
                            .layout(Layout::left_to_right(Align::Center)),
                    );
                    search_ui.add(
                        TextEdit::singleline(&mut tt.detail_search)
                            .id(egui::Id::new(("row-detail-search", idx)))
                            .frame(false)
                            .font(FontId::proportional(12.0))
                            .hint_text("Search for field")
                            .desired_width(f32::INFINITY),
                    );
                    let clear_rect = Rect::from_center_size(
                        egui::pos2(search_rect.right() - 14.0, search_rect.center().y),
                        Vec2::splat(16.0),
                    );
                    if !tt.detail_search.is_empty() {
                        let clear = ui.interact(
                            clear_rect,
                            egui::Id::new(("row-detail-clear", idx)),
                            Sense::click(),
                        );
                        if clear.hovered() {
                            ui.painter().rect_filled(
                                clear_rect,
                                Rounding::same(4.0),
                                p.row_hover,
                            );
                        }
                        widgets::draw_cross(ui.painter(), clear_rect.shrink(5.0), p.faint);
                        if clear.clicked() {
                            tt.detail_search.clear();
                        }
                    }
                    let options = ui.interact(
                        options_rect,
                        egui::Id::new(("row-detail-options", idx)),
                        Sense::click(),
                    );
                    if options.hovered() {
                        ui.painter().rect_filled(
                            options_rect,
                            Rounding::same(6.0),
                            p.row_hover,
                        );
                    }
                    widgets::draw_sliders_icon(
                        ui.painter(),
                        options_rect.shrink(6.0),
                        p.muted,
                    );
                    let popup_id = egui::Id::new(("row-detail-options-popup", idx));
                    if options.clicked() {
                        ui.memory_mut(|memory| memory.toggle_popup(popup_id));
                    }
                    egui::popup::popup_below_widget(
                        ui,
                        popup_id,
                        &options,
                        egui::popup::PopupCloseBehavior::CloseOnClick,
                        |ui| {
                            ui.set_min_width(160.0);
                            if ui
                                .selectable_label(
                                    tt.detail_match == DetailMatch::Fuzzy,
                                    "Fuzzy - default",
                                )
                                .clicked()
                            {
                                tt.detail_match = DetailMatch::Fuzzy;
                            }
                            if ui
                                .selectable_label(
                                    tt.detail_match == DetailMatch::Contains,
                                    "Contains",
                                )
                                .clicked()
                            {
                                tt.detail_match = DetailMatch::Contains;
                            }
                            ui.separator();
                            ui.checkbox(&mut tt.detail_case, "Match case");
                            ui.separator();
                            if ui.button("Hide row detail").clicked() {
                                hide_panel = true;
                            }
                        },
                    );
                    ui.add_space(6.0);

                    ScrollArea::vertical()
                        .id_salt(("row-detail-fields", idx))
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let Some(detail) = tt.detail.as_mut() else {
                                ui.add_space(50.0);
                                ui.vertical_centered(|ui| {
                                    ui.label(
                                        RichText::new("No row selected")
                                            .size(12.6)
                                            .color(p.muted),
                                    );
                                    ui.add_space(3.0);
                                    ui.label(
                                        RichText::new(
                                            "Select a row in the table to edit its fields here.",
                                        )
                                        .size(10.8)
                                        .color(p.faint),
                                    );
                                });
                                return;
                            };
                            if let Some(error) = &detail.error {
                                ui.add_space(2.0);
                                ui.label(RichText::new(error).size(11.0).color(p.red));
                                ui.add_space(2.0);
                            }
                            if !editable {
                                ui.label(
                                    RichText::new(if read_only {
                                        "Read-only mode is on, so the fields cannot be edited."
                                    } else {
                                        "This table has no usable key, so the fields are read-only."
                                    })
                                    .size(10.2)
                                    .color(p.amber),
                                );
                                ui.add_space(4.0);
                            }
                            let needle = tt.detail_search.trim().to_string();
                            let mut shown = 0usize;
                            for (i, field) in detail.fields.iter_mut().enumerate() {
                                if !needle.is_empty()
                                    && !detail_field_matches(
                                        &field.name,
                                        &needle,
                                        tt.detail_match,
                                        tt.detail_case,
                                    )
                                {
                                    continue;
                                }
                                shown += 1;
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(&field.name).size(12.5).color(p.text),
                                    );
                                    if field.primary_key {
                                        ui.label(
                                            RichText::new("PK")
                                                .monospace()
                                                .size(9.0)
                                                .color(p.amber),
                                        );
                                    }
                                    ui.with_layout(
                                        Layout::right_to_left(Align::Center),
                                        |ui| {
                                            if !field.type_label.is_empty() {
                                                ui.label(
                                                    RichText::new(&field.type_label)
                                                        .monospace()
                                                        .size(10.6)
                                                        .color(p.faint),
                                                );
                                            }
                                        },
                                    );
                                });
                                ui.add_space(3.0);

                                let kind = field.editor_kind();
                                let is_blob = matches!(kind, db::FieldEditor::Blob);
                                let is_json = matches!(kind, db::FieldEditor::Json);
                                let multiline =
                                    field.is_multiline() || is_blob || is_json;
                                let box_width = ui.available_width();
                                let inner_width = (box_width - 40.0).max(40.0);
                                // The original's RowDetailTextView auto-sizes to
                                // its wrapped content; measure the galley so long
                                // HTML isn't clipped or overlapped by the next
                                // field.
                                let text_height = if multiline {
                                    ui.fonts(|fonts| {
                                        fonts
                                            .layout(
                                                field.value.clone(),
                                                FontId::proportional(13.0),
                                                p.text,
                                                inner_width,
                                            )
                                            .size()
                                            .y
                                    })
                                } else {
                                    0.0
                                };
                                // Auto-size to the wrapped content, like the
                                // original's RowDetailTextView; the panel's own
                                // scroll area handles the height.
                                let box_h = if multiline {
                                    (text_height + 18.0).clamp(46.0, 1200.0)
                                } else {
                                    38.0
                                };
                                let (rect, _) = ui.allocate_exact_size(
                                    Vec2::new(box_width, box_h),
                                    Sense::hover(),
                                );
                                ui.painter().rect_filled(
                                    rect,
                                    Rounding::same(7.0),
                                    p.editor,
                                );
                                ui.painter().rect_stroke(
                                    rect,
                                    Rounding::same(7.0),
                                    Stroke::new(1.0_f32, p.border),
                                );
                                let right_inset = if matches!(
                                    kind,
                                    db::FieldEditor::Date | db::FieldEditor::DateTime
                                ) {
                                    46.0
                                } else {
                                    30.0
                                };
                                let inner = if multiline {
                                    Rect::from_min_max(
                                        egui::pos2(rect.left() + 10.0, rect.top() + 7.0),
                                        egui::pos2(rect.right() - right_inset, rect.bottom() - 7.0),
                                    )
                                } else {
                                    Rect::from_min_max(
                                        egui::pos2(rect.left() + 10.0, rect.top() + 3.0),
                                        egui::pos2(rect.right() - right_inset, rect.bottom() - 3.0),
                                    )
                                };
                                let mut editor_ui = ui.new_child(
                                    egui::UiBuilder::new()
                                        .max_rect(inner)
                                        .layout(Layout::left_to_right(Align::Center)),
                                );
                                // Keep long values inside their rounded box
                                // and inside the scroll viewport instead of
                                // painting over the grid and status bar.
                                editor_ui.set_clip_rect(inner.intersect(ui.clip_rect()));
                                let id =
                                    egui::Id::new(("row-detail-field", idx, i));
                                let text_color = if field.is_null && field.value.is_empty() {
                                    p.faint
                                } else {
                                    p.text
                                };
                                let json_raw_id =
                                    egui::Id::new(("json-raw", idx, i));
                                let json_raw = ui
                                    .data(|data| data.get_temp::<bool>(json_raw_id).unwrap_or(false));
                                let mut changed_value: Option<String> = None;
                                let mut skip_commit = false;
                                let resp = match &kind {
                                    db::FieldEditor::Bool if editable => {
                                        let mut checked =
                                            widgets::is_truthy(field.value.as_str());
                                        let r = editor_ui.checkbox(&mut checked, "");
                                        if r.changed() {
                                            skip_commit = true;
                                            changed_value = Some(
                                                if checked { "1" } else { "0" }.to_string(),
                                            );
                                        }
                                        r
                                    }
                                    db::FieldEditor::Enum(options) if editable => {
                                        let mut chosen = field.value.clone();
                                        let r = egui::ComboBox::from_id_salt(id.with("enum"))
                                            .selected_text(
                                                RichText::new(&field.value)
                                                    .size(12.0)
                                                    .color(p.text),
                                            )
                                            .width(inner_width)
                                            .show_ui(&mut editor_ui, |ui| {
                                                for option in options {
                                                    ui.selectable_value(
                                                        &mut chosen,
                                                        option.clone(),
                                                        option,
                                                    );
                                                }
                                            })
                                            .response;
                                        if chosen != field.value {
                                            skip_commit = true;
                                            field.value = chosen.clone();
                                            changed_value = Some(chosen);
                                        }
                                        r
                                    }
                                    db::FieldEditor::Blob => {
                                        skip_commit = true;
                                        let bytes = widgets::blob_bytes(&field.value);
                                        editor_ui.add(
                                            egui::Label::new(
                                                RichText::new(widgets::hex_dump(&bytes))
                                                    .monospace()
                                                    .size(10.5)
                                                    .color(p.code),
                                            )
                                            .wrap(),
                                        )
                                    }
                                    db::FieldEditor::Json if !json_raw => {
                                        skip_commit = true;
                                        editor_ui.add(
                                            egui::Label::new(
                                                RichText::new(widgets::pretty_json(&field.value))
                                                    .monospace()
                                                    .size(11.2)
                                                    .color(p.code),
                                            )
                                            .wrap(),
                                        )
                                    }
                                    _ => {
                                        if multiline {
                                            editor_ui.add_sized(
                                                Vec2::new(inner.width(), inner.height()),
                                                TextEdit::multiline(&mut field.value)
                                                    .id(id)
                                                    .frame(false)
                                                    .margin(Margin::ZERO)
                                                    .font(FontId::proportional(13.0))
                                                    .text_color(text_color)
                                                    .interactive(editable)
                                                    .desired_width(f32::INFINITY),
                                            )
                                        } else {
                                            editor_ui.add_sized(
                                                Vec2::new(inner.width(), 22.0),
                                                TextEdit::singleline(&mut field.value)
                                                    .id(id)
                                                    .frame(false)
                                                    .margin(Margin::ZERO)
                                                    .font(FontId::proportional(13.0))
                                                    .text_color(text_color)
                                                    .hint_text(if field.is_null {
                                                        "NULL"
                                                    } else {
                                                        ""
                                                    })
                                                    .vertical_align(Align::Center)
                                                    .interactive(editable)
                                                    .desired_width(f32::INFINITY),
                                            )
                                        }
                                    }
                                };
                                if let Some(value) = changed_value {
                                    queue_detail_commit(
                                        &mut commits,
                                        field.name.clone(),
                                        Some(value),
                                    );
                                }
                                if matches!(
                                    kind,
                                    db::FieldEditor::Date | db::FieldEditor::DateTime
                                ) && editable
                                {
                                    let cal_center = egui::pos2(
                                        rect.right() - 40.0,
                                        rect.center().y,
                                    );
                                    let cal_rect = Rect::from_center_size(
                                        cal_center,
                                        Vec2::splat(18.0),
                                    );
                                    let cal = ui.interact(
                                        cal_rect,
                                        egui::Id::new(("row-detail-cal", idx, i)),
                                        Sense::click(),
                                    );
                                    if cal.hovered() {
                                        ui.painter().rect_filled(
                                            cal_rect,
                                            Rounding::same(4.0),
                                            p.row_hover,
                                        );
                                    }
                                    ui.painter().text(
                                        cal_rect.center(),
                                        Align2::CENTER_CENTER,
                                        "▾",
                                        FontId::proportional(10.0),
                                        p.muted,
                                    );
                                    let popup_id =
                                        egui::Id::new(("row-detail-cal-popup", idx, i));
                                    if cal.clicked() {
                                        ui.memory_mut(|memory| memory.toggle_popup(popup_id));
                                    }
                                    egui::popup::popup_below_widget(
                                        ui,
                                        popup_id,
                                        &cal,
                                        egui::popup::PopupCloseBehavior::CloseOnClickOutside,
                                        |ui| {
                                            ui.set_min_width(228.0);
                                            if widgets::calendar_ui(ui, &p, &mut field.value) {
                                                queue_detail_commit(
                                                    &mut commits,
                                                    field.name.clone(),
                                                    Some(field.value.clone()),
                                                );
                                                ui.memory_mut(|memory| memory.close_popup());
                                            }
                                        },
                                    );
                                }
                                if resp.has_focus() {
                                    ui.painter().rect_stroke(
                                        rect,
                                        Rounding::same(7.0),
                                        Stroke::new(1.4_f32, p.accent),
                                    );
                                } else if resp.hovered() {
                                    ui.painter().rect_stroke(
                                        rect,
                                        Rounding::same(7.0),
                                        Stroke::new(
                                            1.0_f32,
                                            p.muted.gamma_multiply(0.55),
                                        ),
                                    );
                                }

                                // Chevron menu: Set NULL / Set empty / Copy.
                                let chevron_center = if multiline {
                                    egui::pos2(rect.right() - 15.0, rect.top() + 15.0)
                                } else {
                                    egui::pos2(rect.right() - 15.0, rect.center().y)
                                };
                                let chevron_rect =
                                    Rect::from_center_size(chevron_center, Vec2::splat(18.0));
                                let chevron_id =
                                    egui::Id::new(("row-detail-menu", idx, i));
                                let chevron = ui.interact(
                                    chevron_rect,
                                    chevron_id,
                                    Sense::click(),
                                );
                                if chevron.hovered() {
                                    ui.painter().rect_filled(
                                        chevron_rect,
                                        Rounding::same(4.0),
                                        p.row_hover,
                                    );
                                }
                                widgets::draw_chevron_down(
                                    ui.painter(),
                                    chevron_rect.shrink(5.0),
                                    p.faint,
                                );
                                let field_name = field.name.clone();
                                let field_value = field.value.clone();
                                let field_nullable = field.nullable;
                                let mut action: Option<DetailFieldAction> = None;
                                let menu_id = chevron_id.with("popup");
                                if chevron.clicked() {
                                    ui.memory_mut(|memory| memory.toggle_popup(menu_id));
                                }
                                egui::popup::popup_below_widget(
                                    ui,
                                    menu_id,
                                    &chevron,
                                    egui::popup::PopupCloseBehavior::CloseOnClick,
                                    |ui| {
                                        ui.set_min_width(150.0);
                                        if ui
                                            .add_enabled(
                                                editable && field_nullable,
                                                egui::Button::new("Set NULL").frame(false),
                                            )
                                            .clicked()
                                        {
                                            action = Some(DetailFieldAction::SetNull);
                                        }
                                        if ui
                                            .add_enabled(
                                                editable,
                                                egui::Button::new("Set empty").frame(false),
                                            )
                                            .clicked()
                                        {
                                            action = Some(DetailFieldAction::SetEmpty);
                                        }
                                        if is_json {
                                            let raw = ui
                                                .data(|data| {
                                                    data.get_temp::<bool>(json_raw_id)
                                                        .unwrap_or(false)
                                                });
                                            if ui
                                                .add(
                                                    egui::Button::new(if raw {
                                                        "Pretty JSON"
                                                    } else {
                                                        "Edit raw JSON"
                                                    })
                                                    .frame(false),
                                                )
                                                .clicked()
                                            {
                                                ui.data_mut(|data| {
                                                    data.insert_temp(
                                                        json_raw_id,
                                                        !raw,
                                                    )
                                                });
                                            }
                                        }
                                        if is_blob && ui.button("Copy hex").clicked() {
                                            action = Some(DetailFieldAction::Copy(
                                                widgets::hex_encode(&widgets::blob_bytes(
                                                    &field_value,
                                                )),
                                            ));
                                        }
                                        ui.separator();
                                        if ui
                                            .add(
                                                egui::Button::new("Copy value").frame(false),
                                            )
                                            .clicked()
                                        {
                                            action = Some(DetailFieldAction::Copy(
                                                field_value,
                                            ));
                                        }
                                    },
                                );

                                if let Some(action) = action {
                                    match action {
                                        DetailFieldAction::SetNull => {
                                            queue_detail_commit(
                                                &mut commits,
                                                field_name,
                                                None,
                                            );
                                        }
                                        DetailFieldAction::SetEmpty => {
                                            queue_detail_commit(
                                                &mut commits,
                                                field_name,
                                                Some(String::new()),
                                            );
                                        }
                                        DetailFieldAction::Copy(value) => copy = Some(value),
                                    }
                                } else {
                                    // Escape restores the loaded value.
                                    if resp.has_focus()
                                        && ui.input(|i| {
                                            i.key_pressed(egui::Key::Escape)
                                        })
                                    {
                                        field.value =
                                            field.original.clone().unwrap_or_default();
                                        resp.surrender_focus();
                                    }
                                    let cmd_enter = multiline
                                        && resp.has_focus()
                                        && ui.input(|i| {
                                            i.modifiers.command
                                                && i.key_pressed(egui::Key::Enter)
                                        });
                                    if editable
                                        && !skip_commit
                                        && (resp.lost_focus() || cmd_enter)
                                    {
                                        if cmd_enter {
                                            resp.surrender_focus();
                                        }
                                        let current = field.value.clone();
                                        let changed = match &field.original {
                                            None => !current.is_empty(),
                                            Some(original) => &current != original,
                                        };
                                        if changed {
                                            queue_detail_commit(
                                                &mut commits,
                                                field.name.clone(),
                                                Some(current),
                                            );
                                        }
                                    }
                                }
                                ui.add_space(10.0);
                            }
                            if shown == 0 {
                                ui.add_space(30.0);
                                ui.vertical_centered(|ui| {
                                    ui.label(
                                        RichText::new("No matching fields")
                                            .size(11.2)
                                            .color(p.faint),
                                    );
                                });
                            }
                        });
                });
            if hide_panel {
                tt.detail_open = false;
            }
        }

        if let Some(value) = copy {
            self.pending_copy = Some(value);
        }
        for (column, value) in commits {
            self.commit_detail_field(idx, column, value);
        }
    }

    /// Saves any row-detail fields that differ from their loaded values. Used
    /// when the user leaves the tab (sidebar/tab bar/new tab) while a field is
    /// still focused, because no focus-loss event reaches the hidden panel.
    pub(crate) fn flush_row_detail_edits(&mut self, idx: usize) {
        let dirty: Vec<(String, Option<String>)> = {
            let Some(tab) = self.tabs.get(idx) else {
                return;
            };
            let crate::app::TabKind::Table(tt) = &tab.kind else {
                return;
            };
            if !tt.data.editable {
                return;
            }
            let Some(detail) = &tt.detail else {
                return;
            };
            if detail.key_column.is_empty() || detail.key_value.is_empty() {
                return;
            }
            detail
                .fields
                .iter()
                .filter_map(|field| {
                    let changed = match &field.original {
                        None => !field.value.is_empty(),
                        Some(original) => &field.value != original,
                    };
                    changed.then(|| (field.name.clone(), Some(field.value.clone())))
                })
                .collect()
        };
        for (column, value) in dirty {
            self.commit_detail_field(idx, column, value);
        }
    }

    /// Opens the full-width editor for JSON and binary values that do not fit
    /// a grid cell.
    pub(crate) fn open_value_editor(&mut self, idx: usize, row: usize, col: usize) {
        let Some(tab) = self.tabs.get(idx) else {
            return;
        };
        let tab_id = tab.id;
        let crate::app::TabKind::Table(tt) = &tab.kind else {
            return;
        };
        let Some(column) = tt.data.columns.get(col).cloned() else {
            return;
        };
        let Some(key_column) = tt.data.key_column.clone() else {
            self.push_toast(
                "This table has no usable key, so the value is read-only.".to_string(),
                ToastKind::Error,
            );
            return;
        };
        let key_value = tt
            .data
            .rowids
            .get(row)
            .cloned()
            .flatten()
            .unwrap_or_default();
        let text = tt
            .data
            .rows
            .get(row)
            .and_then(|values| values.get(col))
            .cloned()
            .flatten()
            .unwrap_or_default();
        let kind = column_editor_kind(tt, col);
        let before = tt
            .data
            .rows
            .get(row)
            .and_then(|values| values.get(col))
            .cloned()
            .flatten();
        self.value_editor = Some(crate::app::ValueEditor {
            tab_id,
            table: tt.table.clone(),
            key_column,
            key_value,
            column,
            kind,
            text,
            before,
            error: None,
        });
    }

    /// Full-width value window for JSON and blob cells.
    pub(crate) fn value_editor_window(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let Some(mut editor) = self.value_editor.take() else {
            return;
        };
        let mut close = false;
        let mut save = false;
        let mut copy: Option<String> = None;
        let is_blob = matches!(editor.kind, db::FieldEditor::Blob);
        egui::Window::new(format!("{} · {}", editor.column, editor.kind.label()))
            .collapsible(false)
            .resizable(true)
            .default_width(620.0)
            .default_height(460.0)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(Rounding::same(10.0))
                    .inner_margin(Margin::symmetric(16.0, 14.0)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(editor.table.display())
                            .size(11.0)
                            .color(p.muted),
                    );
                    ui.label(
                        RichText::new(format!("key {}", editor.key_value))
                            .monospace()
                            .size(10.4)
                            .color(p.faint),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if is_blob {
                            if ui.button("Copy text").clicked() {
                                copy = Some(widgets::blob_text_preview(
                                    &widgets::blob_bytes(&editor.text),
                                ));
                            }
                            if ui.button("Copy hex").clicked() {
                                copy = Some(widgets::hex_encode(&widgets::blob_bytes(
                                    &editor.text,
                                )));
                            }
                        } else if ui.button("Format").clicked() {
                            editor.text = widgets::pretty_json(&editor.text);
                        }
                    });
                });
                ui.add_space(6.0);
                if is_blob {
                    let bytes = widgets::blob_bytes(&editor.text);
                    ui.label(
                        RichText::new(format!(
                            "{} bytes · read-only preview (binary editing is not supported yet)",
                            bytes.len()
                        ))
                        .size(10.4)
                        .color(p.faint),
                    );
                    ui.add_space(4.0);
                    ScrollArea::vertical()
                        .id_salt("blob-hex-scroll")
                        .max_height(280.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(widgets::hex_dump(&bytes))
                                    .monospace()
                                    .size(10.8)
                                    .color(p.code),
                            );
                        });
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("Text preview").size(10.6).strong().color(p.faint),
                    );
                    ui.add_space(2.0);
                    ScrollArea::vertical()
                        .id_salt("blob-text-scroll")
                        .max_height(110.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(widgets::blob_text_preview(&bytes))
                                    .monospace()
                                    .size(10.8)
                                    .color(p.text),
                            );
                        });
                } else {
                    ScrollArea::vertical()
                        .id_salt("value-editor-scroll")
                        .max_height(300.0)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.add_sized(
                                Vec2::new(ui.available_width(), 280.0),
                                TextEdit::multiline(&mut editor.text)
                                    .frame(true)
                                    .code_editor()
                                    .desired_width(f32::INFINITY)
                                    .font(TextStyle::Monospace),
                            );
                        });
                }
                if let Some(error) = &editor.error {
                    ui.add_space(4.0);
                    ui.label(RichText::new(error).size(11.0).color(p.red));
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if !is_blob {
                        let primary = egui::Button::new(
                            RichText::new("Save").size(12.0).color(Color32::WHITE),
                        )
                        .fill(p.accent)
                        .rounding(Rounding::same(6.0))
                        .min_size(Vec2::new(90.0, 27.0));
                        if ui.add(primary).clicked() {
                            save = true;
                        }
                    }
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Close").size(12.0).color(p.muted))
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE),
                        )
                        .clicked()
                    {
                        close = true;
                    }
                });
            });

        if let Some(text) = copy {
            self.pending_copy = Some(text);
        }
        if save {
            let table = editor.table.sql();
            let result = match &self.live {
                Some(live) => live.db.update_cell(
                    &table,
                    &editor.column,
                    &editor.key_column,
                    &editor.key_value,
                    Some(editor.text.as_str()),
                ),
                None => Err("The connection is closed.".to_string()),
            };
            match result {
                Ok(affected) => {
                    self.record_edit(
                        &editor.table,
                        &editor.key_column,
                        &editor.key_value,
                        &editor.column,
                        editor.before.clone(),
                        Some(editor.text.clone()),
                    );
                    if let Some(index) =
                        self.tabs.iter().position(|tab| tab.id == editor.tab_id)
                    {
                        self.refresh_table_tab(index);
                    }
                    self.push_toast(
                        format!(
                            "Updated {} row{}",
                            affected,
                            if affected == 1 { "" } else { "s" }
                        ),
                        ToastKind::Info,
                    );
                }
                Err(error) => {
                    editor.error = Some(error);
                    self.value_editor = Some(editor);
                }
            }
        } else if !close {
            self.value_editor = Some(editor);
        }
    }

    /// Persists one field edited in the row detail panel, then reloads the
    /// page and rebuilds the vertical form with the stored value.
    fn commit_detail_field(&mut self, idx: usize, column: String, value: Option<String>) {
        let driver = self
            .live
            .as_ref()
            .map(|live| live.db.driver)
            .unwrap_or(Driver::Sqlite);
        let target = {
            let Some(tab) = self.tabs.get(idx) else {
                return;
            };
            let crate::app::TabKind::Table(tt) = &tab.kind else {
                return;
            };
            let Some(detail) = &tt.detail else {
                return;
            };
            if detail.key_column.is_empty() || detail.key_value.is_empty() {
                return;
            }
            (
                tt.table.sql(),
                detail.key_column.clone(),
                detail.key_value.clone(),
                detail.row,
            )
        };
        let target_table = self
            .tabs
            .get(idx)
            .and_then(|tab| match &tab.kind {
                crate::app::TabKind::Table(tt) => Some(tt.table.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let result = match &self.live {
            Some(live) => live.db.update_field(
                &target.0,
                &column,
                &target.1,
                &target.2,
                value.as_deref(),
            ),
            None => Err("The connection is closed.".to_string()),
        };
        // If the edited column is the row key itself (MySQL integer primary
        // key), the refreshed page carries the new key, not the old one.
        let new_key = if column == target.1 {
            value.clone()
        } else {
            None
        };
        let before = {
            let mut before = None;
            if let Some(tab) = self.tabs.get(idx) {
                if let crate::app::TabKind::Table(tt) = &tab.kind {
                    if let Some(detail) = &tt.detail {
                        before = detail
                            .fields
                            .iter()
                            .find(|field| field.name == column)
                            .and_then(|field| field.original.clone());
                    }
                }
            }
            before
        };
        match result {
            Ok(affected) => {
                self.record_edit(
                    &target_table,
                    &target.1,
                    &target.2,
                    &column,
                    before,
                    value.clone(),
                );
                self.refresh_table_tab(idx);
                if let Some(tab) = self.tabs.get_mut(idx) {
                    if let crate::app::TabKind::Table(tt) = &mut tab.kind {
                        if !tt.data.rows.is_empty() {
                            // Prefer the row whose key is unchanged (rowid on
                            // SQLite, integer primary key on MySQL) so a
                            // replayed page cannot select a different record.
                            // PostgreSQL ctid changes on UPDATE, so fall back
                            // to the old index there.
                            let row = tt
                                .data
                                .rowids
                                .iter()
                                .position(|key| match &new_key {
                                    Some(new_key) => key.as_deref() == Some(new_key.as_str()),
                                    None => key.as_deref() == Some(target.2.as_str()),
                                })
                                .unwrap_or_else(|| target.3.min(tt.data.rows.len() - 1));
                            let col = tt
                                .data
                                .columns
                                .iter()
                                .position(|name| name == &column)
                                .unwrap_or(0);
                            tt.selected.set_cell(row, col);
                            tt.detail = tt.build_row_detail(row, driver);
                        }
                    }
                }
                if affected == 0 {
                    if driver == Driver::MySql {
                        // MySQL reports 0 affected rows when the new value
                        // equals the old one (changed-rows semantics), so this
                        // is not an error.
                        self.push_toast(
                            format!("Saved {column} (no change)"),
                            ToastKind::Info,
                        );
                    } else {
                        // SQLite/PostgreSQL count a matched row, so zero means
                        // the record disappeared between load and save.
                        self.push_toast(
                            "No row was updated; it may have been deleted.".to_string(),
                            ToastKind::Error,
                        );
                    }
                } else {
                    self.push_toast(format!("Saved {column}"), ToastKind::Info);
                }
            }
            Err(error) => {
                if let Some(tab) = self.tabs.get_mut(idx) {
                    if let crate::app::TabKind::Table(tt) = &mut tab.kind {
                        if let Some(detail) = &mut tt.detail {
                            detail.error = Some(error);
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn open_row_editor(&mut self, idx: usize) {
        let editor = {
            let Some(tab) = self.tabs.get(idx) else {
                return;
            };
            let crate::app::TabKind::Table(tt) = &tab.kind else {
                return;
            };
            Some(RowEditor {
                tab: idx,
                table: tt.table.clone(),
                fields: tt
                    .structure
                    .iter()
                    .map(|c| RowField {
                        name: c.name.clone(),
                        decl_type: c.decl_type.clone(),
                        primary_key: c.primary_key,
                        value: String::new(),
                    })
                    .collect(),
                error: None,
            })
        };
        if let Some(editor) = editor {
            self.row_editor = Some(editor);
        }
    }

    pub(crate) fn open_column_editor(&mut self, idx: usize, column: Option<String>) {
        let editor = {
            let Some(tab) = self.tabs.get(idx) else {
                return;
            };
            let crate::app::TabKind::Table(tt) = &tab.kind else {
                return;
            };
            match column {
                Some(name) => {
                    tt.structure
                        .iter()
                        .find(|c| c.name == name)
                        .map(|meta| ColumnEditor {
                            tab: idx,
                            table: tt.table.clone(),
                            original: Some(meta.name.clone()),
                            name: meta.name.clone(),
                            decl_type: meta.decl_type.clone(),
                            not_null: meta.not_null,
                            default_value: meta.default_value.clone().unwrap_or_default(),
                            original_type: meta.decl_type.clone(),
                            original_not_null: meta.not_null,
                            error: None,
                        })
                }
                None => Some(ColumnEditor {
                    tab: idx,
                    table: tt.table.clone(),
                    original: None,
                    name: String::new(),
                    decl_type: "TEXT".into(),
                    not_null: false,
                    default_value: String::new(),
                    original_type: String::new(),
                    original_not_null: false,
                    error: None,
                }),
            }
        };
        if let Some(editor) = editor {
            self.column_editor = Some(editor);
        }
    }

    pub(crate) fn confirm_delete_column(&mut self, idx: usize, column: String) {
        let table = {
            let Some(tab) = self.tabs.get(idx) else {
                return;
            };
            let crate::app::TabKind::Table(tt) = &tab.kind else {
                return;
            };
            tt.table.clone()
        };
        self.column_delete = Some(ColumnDelete {
            tab: idx,
            table,
            column,
            error: None,
        });
    }

    pub(crate) fn confirm_delete_row(
        &mut self,
        idx: usize,
        table: db::TableId,
        key_column: String,
        key_value: String,
        row_number: usize,
    ) {
        self.row_delete = Some(RowDelete {
            tab: idx,
            table,
            key_column,
            key_value,
            row_number,
            error: None,
        });
    }

    pub(crate) fn row_delete_window(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let Some(mut pending) = self.row_delete.take() else {
            return;
        };
        let mut close = false;
        let mut delete = false;

        egui::Window::new("Delete record")
            .collapsible(false)
            .resizable(false)
            .default_width(380.0)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(Rounding::same(10.0))
                    .inner_margin(Margin::symmetric(18.0, 16.0)),
            )
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(
                    RichText::new(format!("Delete row {}?", pending.row_number))
                        .size(13.0)
                        .color(p.text),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new("The record will be removed from the table.")
                        .size(11.0)
                        .color(p.muted),
                );
                if let Some(err) = &pending.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(err).size(11.2).color(p.red));
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Delete").size(12.0).color(Color32::WHITE),
                            )
                            .fill(p.red)
                            .rounding(Rounding::same(6.0))
                            .min_size(Vec2::new(78.0, 28.0)),
                        )
                        .clicked()
                    {
                        delete = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Cancel").size(12.0).color(p.muted))
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE),
                        )
                        .clicked()
                    {
                        close = true;
                    }
                });
            });

        if delete {
            let result = if let Some(live) = &self.live {
                live.db.delete_row(
                    &pending.table.sql(),
                    &pending.key_column,
                    &pending.key_value,
                )
            } else {
                Err("The connection is closed.".to_string())
            };
            match result {
                Ok(0) => {
                    pending.error = Some("No row was deleted. Refresh and try again.".to_string());
                    self.row_delete = Some(pending);
                }
                Ok(_) => {
                    self.refresh_table_tab(pending.tab);
                    self.push_toast("Record deleted".to_string(), ToastKind::Info);
                }
                Err(e) => {
                    pending.error = Some(e);
                    self.row_delete = Some(pending);
                }
            }
        } else if !close {
            self.row_delete = Some(pending);
        }
    }

    pub(crate) fn connection_delete_window(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let Some(pending) = self.connection_delete.take() else {
            return;
        };
        let mut close = false;
        let mut delete = false;

        egui::Window::new("Delete connection")
            .collapsible(false)
            .resizable(false)
            .default_width(380.0)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(Rounding::same(10.0))
                    .inner_margin(Margin::symmetric(18.0, 16.0)),
            )
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(
                    RichText::new(format!("Delete “{}”?", pending.name))
                        .size(13.0)
                        .color(p.text),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "The saved connection will be removed. The database itself is not deleted.",
                    )
                    .size(11.0)
                    .color(p.muted),
                );
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Delete").size(12.0).color(Color32::WHITE),
                            )
                            .fill(p.red)
                            .rounding(Rounding::same(6.0))
                            .min_size(Vec2::new(78.0, 28.0)),
                        )
                        .clicked()
                    {
                        delete = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Cancel").size(12.0).color(p.muted))
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE),
                        )
                        .clicked()
                    {
                        close = true;
                    }
                });
            });

        if delete {
            let was_live = self.live.as_ref().map(|l| l.saved.id) == Some(pending.id);
            self.store.delete_connection(pending.id);
            if was_live {
                self.disconnect();
            }
            self.push_toast("Connection deleted".to_string(), ToastKind::Info);
        } else if !close {
            self.connection_delete = Some(pending);
        }
    }

    pub(crate) fn clone_table_window(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let Some(mut pending) = self.clone_table.take() else {
            return;
        };
        let driver = self.live.as_ref().map(|live| live.db.driver);
        let clone_note = if driver == Some(Driver::Sqlite) {
            "The new table gets the same columns, primary keys and constraints, plus all rows. Secondary indexes are not copied."
        } else {
            "The new table gets the same structure and indexes, plus all rows."
        };
        let mut close = false;
        let mut clone = false;

        let response = egui::Modal::new(egui::Id::new("clone-table"))
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(Rounding::same(10.0))
                    .inner_margin(Margin::symmetric(18.0, 16.0)),
            )
            .show(ctx, |ui| {
                ui.set_width(400.0);
                ui.label(
                    RichText::new(format!("Clone {}", pending.source))
                        .size(13.0)
                        .color(p.text),
                );
                ui.add_space(2.0);
                ui.label(
                    RichText::new(clone_note)
                        .size(10.8)
                        .color(p.muted),
                );
                ui.add_space(10.0);
                ui.label(RichText::new("New table name").size(10.4).color(p.faint));
                let resp = ui.add_sized(
                    Vec2::new(ui.available_width(), 26.0),
                    TextEdit::singleline(&mut pending.name).hint_text("table_copy"),
                );
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    clone = true;
                }
                if let Some(err) = &pending.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(err).size(11.2).color(p.red));
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Clone").size(12.0).color(Color32::WHITE),
                            )
                            .fill(p.accent)
                            .rounding(Rounding::same(6.0))
                            .min_size(Vec2::new(78.0, 28.0)),
                        )
                        .clicked()
                    {
                        clone = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Cancel").size(12.0).color(p.muted))
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE),
                        )
                        .clicked()
                    {
                        close = true;
                    }
                });
            });
        if response.should_close() {
            close = true;
        }

        if clone {
            let result = if let Some(live) = &self.live {
                live.db
                    .clone_table(&pending.source.sql(), pending.name.trim())
            } else {
                Err("The connection is closed.".to_string())
            };
            match result {
                Ok(()) => {
                    let name = pending.name.trim().to_string();
                    let target =
                        db::TableId::with_schema(pending.source.schema.clone(), name.clone());
                    let display = target.display();
                    self.refresh_schema();
                    self.open_table_tab(target);
                    self.push_toast(format!("Table cloned as {display}"), ToastKind::Info);
                }
                Err(error) => {
                    pending.error = Some(error);
                    self.clone_table = Some(pending);
                }
            }
        } else if !close {
            self.clone_table = Some(pending);
        }
    }

    pub(crate) fn group_editor_window(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let Some(mut editor) = self.group_editor.take() else {
            return;
        };
        let mut close = false;
        let mut save = false;

        let response = egui::Modal::new(egui::Id::new("connection-group-editor"))
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(Rounding::same(10.0))
                    .inner_margin(Margin::symmetric(18.0, 16.0)),
            )
            .show(ctx, |ui| {
                ui.set_width(360.0);
                ui.label(
                    RichText::new(if editor.id.is_some() {
                        "Rename group"
                    } else {
                        "New group"
                    })
                    .size(15.0)
                    .strong()
                    .color(p.text),
                );
                ui.add_space(3.0);
                ui.label(
                    RichText::new(
                        "Connections in this group are shown together on the connections screen.",
                    )
                    .size(10.8)
                    .color(p.muted),
                );
                ui.add_space(10.0);
                let resp = ui.add_sized(
                    Vec2::new(ui.available_width(), 26.0),
                    TextEdit::singleline(&mut editor.name).hint_text("Project name"),
                );
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    save = true;
                }
                if let Some(error) = &editor.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(error).size(11.2).color(p.red));
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Save").size(12.0).color(Color32::WHITE),
                            )
                            .fill(p.accent)
                            .rounding(Rounding::same(6.0))
                            .min_size(Vec2::new(72.0, 28.0)),
                        )
                        .clicked()
                    {
                        save = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Cancel").size(12.0).color(p.muted))
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE),
                        )
                        .clicked()
                    {
                        close = true;
                    }
                });
            });
        if response.should_close() {
            close = true;
        }

        if save {
            let name = editor.name.trim().to_string();
            let duplicate = self.store.groups.iter().any(|group| {
                group.name.eq_ignore_ascii_case(&name)
                    && Some(group.id) != editor.id
            });
            if name.is_empty() {
                editor.error = Some("Give the group a name.".to_string());
                self.group_editor = Some(editor);
            } else if duplicate {
                editor.error = Some("A group with that name already exists.".to_string());
                self.group_editor = Some(editor);
            } else if let Some(id) = editor.id {
                self.store.rename_group(id, &name);
                self.push_toast(format!("Group renamed to {name}"), ToastKind::Info);
            } else {
                self.store.add_group(&name);
                self.push_toast(format!("Group {name} created"), ToastKind::Info);
            }
        } else if !close {
            self.group_editor = Some(editor);
        }
    }

    pub(crate) fn group_delete_window(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let Some(pending) = self.group_delete.take() else {
            return;
        };
        let mut close = false;
        let mut delete = false;

        let response = egui::Modal::new(egui::Id::new("connection-group-delete"))
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(Rounding::same(10.0))
                    .inner_margin(Margin::symmetric(18.0, 16.0)),
            )
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(
                    RichText::new(format!("Delete group “{}”?", pending.name))
                        .size(13.5)
                        .strong()
                        .color(p.text),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "The connections in this group stay saved; they move back to Ungrouped.",
                    )
                    .size(11.0)
                    .color(p.muted),
                );
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Delete group")
                                    .size(12.0)
                                    .color(Color32::WHITE),
                            )
                            .fill(p.red)
                            .rounding(Rounding::same(6.0))
                            .min_size(Vec2::new(104.0, 28.0)),
                        )
                        .clicked()
                    {
                        delete = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Cancel").size(12.0).color(p.muted))
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE),
                        )
                        .clicked()
                    {
                        close = true;
                    }
                });
            });
        if response.should_close() {
            close = true;
        }
        if delete {
            self.store.delete_group(pending.id);
            self.push_toast(
                format!("Group {} deleted; its connections are ungrouped", pending.name),
                ToastKind::Info,
            );
        } else if !close {
            self.group_delete = Some(pending);
        }
    }

    pub(crate) fn truncate_table_window(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let Some(mut pending) = self.truncate_table.take() else {
            return;
        };
        let mut close = false;
        let mut truncate = false;

        let response = egui::Modal::new(egui::Id::new("truncate-table"))
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(Rounding::same(10.0))
                    .inner_margin(Margin::symmetric(18.0, 16.0)),
            )
            .show(ctx, |ui| {
                ui.set_width(400.0);
                ui.label(
                    RichText::new(format!("Remove all rows from {}?", pending.table))
                        .size(13.0)
                        .color(p.text),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "The table structure and columns stay. Every row is deleted and this cannot be undone.",
                    )
                    .size(11.0)
                    .color(p.muted),
                );
                if let Some(err) = &pending.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(err).size(11.2).color(p.red));
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Truncate").size(12.0).color(Color32::WHITE),
                            )
                            .fill(p.red)
                            .rounding(Rounding::same(6.0))
                            .min_size(Vec2::new(88.0, 28.0)),
                        )
                        .clicked()
                    {
                        truncate = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Cancel").size(12.0).color(p.muted))
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE),
                        )
                        .clicked()
                    {
                        close = true;
                    }
                });
            });
        if response.should_close() {
            close = true;
        }

        if truncate {
            let result = if let Some(live) = &self.live {
                live.db.truncate_table(&pending.table.sql())
            } else {
                Err("The connection is closed.".to_string())
            };
            match result {
                Ok(()) => {
                    self.refresh_after_import();
                    self.push_toast(
                        format!("Table {} truncated", pending.table),
                        ToastKind::Info,
                    );
                }
                Err(error) => {
                    pending.error = Some(error);
                    self.truncate_table = Some(pending);
                }
            }
        } else if !close {
            self.truncate_table = Some(pending);
        }
    }

    pub(crate) fn delete_table_window(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let Some(mut pending) = self.delete_table.take() else {
            return;
        };
        let mut close = false;
        let mut delete = false;

        let response = egui::Modal::new(egui::Id::new("delete-table"))
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(Rounding::same(10.0))
                    .inner_margin(Margin::symmetric(18.0, 16.0)),
            )
            .show(ctx, |ui| {
                ui.set_width(400.0);
                ui.label(
                    RichText::new(format!("Delete {}?", pending.table))
                        .size(13.0)
                        .color(p.text),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "The table, its structure and all rows will be removed. This cannot be undone.",
                    )
                    .size(11.0)
                    .color(p.muted),
                );
                if let Some(err) = &pending.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(err).size(11.2).color(p.red));
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Delete").size(12.0).color(Color32::WHITE),
                            )
                            .fill(p.red)
                            .rounding(Rounding::same(6.0))
                            .min_size(Vec2::new(78.0, 28.0)),
                        )
                        .clicked()
                    {
                        delete = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Cancel").size(12.0).color(p.muted))
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE),
                        )
                        .clicked()
                    {
                        close = true;
                    }
                });
            });
        if response.should_close() {
            close = true;
        }

        if delete {
            let result = if let Some(live) = &self.live {
                live.db.drop_table(&pending.table.sql())
            } else {
                Err("The connection is closed.".to_string())
            };
            match result {
                Ok(()) => {
                    let table = pending.table.clone();
                    if let Some(idx) = self.tabs.iter().position(|tab| {
                        matches!(&tab.kind, crate::app::TabKind::Table(tt) if tt.table == table)
                    }) {
                        // The table is gone, so there is nothing left to save;
                        // clear the detail form before closing the tab.
                        if let Some(tab) = self.tabs.get_mut(idx) {
                            if let crate::app::TabKind::Table(tt) = &mut tab.kind {
                                tt.detail = None;
                            }
                        }
                        self.close_tab(idx);
                    }
                    self.refresh_schema();
                    self.push_toast(format!("Table {table} deleted"), ToastKind::Info);
                }
                Err(error) => {
                    pending.error = Some(error);
                    self.delete_table = Some(pending);
                }
            }
        } else if !close {
            self.delete_table = Some(pending);
        }
    }

    pub(crate) fn row_editor_window(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let Some(mut editor) = self.row_editor.take() else {
            return;
        };
        let mut close = false;
        let mut insert = false;

        egui::Window::new("Add record")
            .collapsible(false)
            .resizable(true)
            .default_width(460.0)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(Rounding::same(10.0))
                    .inner_margin(Margin::symmetric(18.0, 16.0)),
            )
            .show(ctx, |ui| {
                ui.set_width(460.0);
                ui.label(
                    RichText::new(editor.table.display())
                        .size(14.0)
                        .strong()
                        .color(p.text),
                );
                ui.add_space(2.0);
                ui.label(
                    RichText::new("Leave a field empty to use its default value (or NULL).")
                        .size(10.8)
                        .color(p.muted),
                );
                ui.add_space(10.0);
                ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    for field in &mut editor.fields {
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(&field.name)
                                    .monospace()
                                    .size(11.6)
                                    .color(p.text),
                            );
                            if !field.decl_type.is_empty() {
                                widgets::chip(ui, &p, &field.decl_type, p.purple);
                            }
                            if field.primary_key {
                                widgets::chip(ui, &p, "PK", p.amber);
                            }
                        });
                        ui.add(
                            TextEdit::singleline(&mut field.value)
                                .desired_width(f32::INFINITY)
                                .hint_text("value"),
                        );
                        ui.add_space(2.0);
                    }
                });
                if let Some(err) = &editor.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(err).size(11.2).color(p.red));
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Insert").size(12.0).color(Color32::WHITE),
                            )
                            .fill(p.accent)
                            .rounding(Rounding::same(6.0))
                            .min_size(Vec2::new(78.0, 28.0)),
                        )
                        .clicked()
                    {
                        insert = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Cancel").size(12.0).color(p.muted))
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE),
                        )
                        .clicked()
                    {
                        close = true;
                    }
                });
            });

        if insert {
            let values: Vec<(String, String)> = editor
                .fields
                .iter()
                .filter(|f| !f.value.trim().is_empty())
                .map(|f| (f.name.clone(), f.value.clone()))
                .collect();
            let result = if let Some(live) = &self.live {
                live.db.insert_row(&editor.table.sql(), &values)
            } else {
                Err("The connection is closed.".to_string())
            };
            match result {
                Ok(()) => {
                    self.refresh_table_tab(editor.tab);
                    self.push_toast("Record added".to_string(), ToastKind::Info);
                }
                Err(e) => {
                    editor.error = Some(e);
                    self.row_editor = Some(editor);
                }
            }
        } else if !close {
            self.row_editor = Some(editor);
        }
    }

    pub(crate) fn column_editor_window(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let Some(mut editor) = self.column_editor.take() else {
            return;
        };
        let mut close = false;
        let mut save = false;
        let editing = editor.original.is_some();

        egui::Window::new(if editing { "Edit column" } else { "Add column" })
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(Rounding::same(10.0))
                    .inner_margin(Margin::symmetric(18.0, 16.0)),
            )
            .show(ctx, |ui| {
                ui.set_width(420.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(editor.table.display())
                            .size(14.0)
                            .strong()
                            .color(p.text),
                    );
                    widgets::chip(ui, &p, "column", p.purple);
                });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Name").size(11.0).color(p.muted));
                    ui.add(
                        TextEdit::singleline(&mut editor.name)
                            .desired_width(f32::INFINITY)
                            .hint_text("column_name"),
                    );
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Type").size(11.0).color(p.muted));
                    ui.add(
                        TextEdit::singleline(&mut editor.decl_type)
                            .desired_width(f32::INFINITY)
                            .hint_text("TEXT, INTEGER, VARCHAR(255)…"),
                    );
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Default").size(11.0).color(p.muted));
                    ui.add(
                        TextEdit::singleline(&mut editor.default_value)
                            .desired_width(f32::INFINITY)
                            .hint_text("empty, 0, CURRENT_TIMESTAMP…"),
                    );
                });
                ui.add_space(8.0);
                ui.checkbox(
                    &mut editor.not_null,
                    RichText::new("Not null").size(11.2).color(p.text),
                );
                if editing && self.live.as_ref().map(|l| l.db.driver) == Some(Driver::Sqlite) {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(
                            "SQLite can rename this column; type and nullability stay as they are.",
                        )
                        .size(10.4)
                        .color(p.faint),
                    );
                }
                if let Some(err) = &editor.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(err).size(11.2).color(p.red));
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(if editing { "Save" } else { "Add" })
                                    .size(12.0)
                                    .color(Color32::WHITE),
                            )
                            .fill(p.accent)
                            .rounding(Rounding::same(6.0))
                            .min_size(Vec2::new(78.0, 28.0)),
                        )
                        .clicked()
                    {
                        save = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Cancel").size(12.0).color(p.muted))
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE),
                        )
                        .clicked()
                    {
                        close = true;
                    }
                });
            });

        if save {
            let name = editor.name.trim().to_string();
            let result = if name.is_empty() {
                Err("Give the column a name.".to_string())
            } else if let Some(live) = &self.live {
                match &editor.original {
                    None => live.db.add_column(
                        &editor.table.sql(),
                        &name,
                        &editor.decl_type,
                        editor.not_null,
                        &editor.default_value,
                    ),
                    Some(original) => {
                        let type_changed = editor.decl_type.trim() != editor.original_type.trim();
                        let null_changed = editor.not_null != editor.original_not_null;
                        if live.db.driver == Driver::Sqlite && (type_changed || null_changed) {
                            Err("SQLite can rename a column, but changing its type or nullability needs a table rebuild."
                                .to_string())
                        } else {
                            live.db
                                .rename_column(&editor.table.sql(), original, &name)
                                .and_then(|()| {
                                    if !editor.decl_type.trim().is_empty()
                                        && (type_changed || null_changed)
                                    {
                                        live.db.change_column(
                                            &editor.table.sql(),
                                            &name,
                                            &editor.decl_type,
                                            editor.not_null,
                                        )
                                    } else {
                                        Ok(())
                                    }
                                })
                        }
                    }
                }
            } else {
                Err("The connection is closed.".to_string())
            };
            match result {
                Ok(()) => {
                    self.refresh_table_tab(editor.tab);
                    self.push_toast(
                        if editing {
                            "Column updated".to_string()
                        } else {
                            "Column added".to_string()
                        },
                        ToastKind::Info,
                    );
                }
                Err(e) => {
                    editor.error = Some(e);
                    self.column_editor = Some(editor);
                }
            }
        } else if !close {
            self.column_editor = Some(editor);
        }
    }

    pub(crate) fn column_delete_window(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let Some(mut pending) = self.column_delete.take() else {
            return;
        };
        let mut close = false;
        let mut delete = false;

        egui::Window::new("Delete column")
            .collapsible(false)
            .resizable(false)
            .default_width(380.0)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(Rounding::same(10.0))
                    .inner_margin(Margin::symmetric(18.0, 16.0)),
            )
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(
                    RichText::new(format!("Delete column {}?", pending.column))
                        .size(13.0)
                        .color(p.text),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Its data will be removed from every row.")
                        .size(11.0)
                        .color(p.muted),
                );
                if let Some(err) = &pending.error {
                    ui.add_space(6.0);
                    ui.label(RichText::new(err).size(11.2).color(p.red));
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new("Delete").size(12.0).color(Color32::WHITE),
                            )
                            .fill(p.red)
                            .rounding(Rounding::same(6.0))
                            .min_size(Vec2::new(78.0, 28.0)),
                        )
                        .clicked()
                    {
                        delete = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Cancel").size(12.0).color(p.muted))
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE),
                        )
                        .clicked()
                    {
                        close = true;
                    }
                });
            });

        if delete {
            let result = if let Some(live) = &self.live {
                live.db
                    .drop_column(&pending.table.sql(), &pending.column)
            } else {
                Err("The connection is closed.".to_string())
            };
            match result {
                Ok(()) => {
                    self.refresh_table_tab(pending.tab);
                    self.push_toast("Column deleted".to_string(), ToastKind::Info);
                }
                Err(e) => {
                    pending.error = Some(e);
                    self.column_delete = Some(pending);
                }
            }
        } else if !close {
            self.column_delete = Some(pending);
        }
    }

    /// Limit/Offset popover recovered from `PagingPopover` (0x100532758) and
    /// `sub_10071b5e4` (0x10071b5e4).
    pub(crate) fn paging_window(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let idx = self.active;
        let mut applied = false;

        if let Some(tab) = self.tabs.get_mut(idx) {
            if let crate::app::TabKind::Table(tt) = &mut tab.kind {
                if tt.paging_open {
                    egui::Window::new("Page settings")
                        .collapsible(false)
                        .resizable(false)
                        .show(ctx, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Limit").size(11.0).color(p.muted));
                                ui.add(
                                    TextEdit::singleline(&mut tt.limit_text).desired_width(110.0),
                                );
                            });
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Offset").size(11.0).color(p.muted));
                                ui.add(
                                    TextEdit::singleline(&mut tt.offset_text).desired_width(110.0),
                                );
                            });
                            if let Some(err) = &tt.paging_error {
                                ui.label(RichText::new(err).size(10.8).color(p.red));
                            }
                            ui.horizontal(|ui| {
                                if ui.button("Apply").clicked() {
                                    // `sub_100532a80` (0x100532a80) parses both
                                    // fields as signed integers and turns text
                                    // that is not a number into 0 before the
                                    // validation in `sub_10071b5e4` runs.
                                    let limit = db::parse_paging_text(&tt.limit_text);
                                    let offset = db::parse_paging_text(&tt.offset_text);
                                    match db::validate_paging(limit, offset, tt.data.total) {
                                        Ok(()) => {
                                            tt.limit = limit as usize;
                                            tt.offset = offset as usize;
                                            tt.paging_open = false;
                                            tt.paging_error = None;
                                            applied = true;
                                        }
                                        Err(message) => {
                                            tt.paging_error = Some(message.to_string());
                                        }
                                    }
                                }
                                if ui.button("Cancel").clicked() {
                                    tt.paging_open = false;
                                    tt.paging_error = None;
                                }
                            });
                        });
                }
            }
        }

        if applied {
            self.refresh_table_tab(idx);
        }
    }

/// One saved connection card on the welcome screen. `Connect` is the only
/// visible button; edit/duplicate/delete/export live in the right-click menu.
fn connection_card(
    ui: &mut Ui,
    p: &Palette,
    saved: &SavedConnection,
) -> (bool, Option<ConnectionMenuAction>) {
    let mut connect = false;
    let mut action: Option<ConnectionMenuAction> = None;
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 58.0), Sense::click());
    if resp.clicked() {
        // The whole card is a connect target, so a click near (not exactly on)
        // the button still connects.
        connect = true;
    }
    let bg = if resp.hovered() { p.row_hover } else { p.card };
    ui.painter()
        .rect_filled(rect, Rounding::same(10.0), bg);
    ui.painter().rect_stroke(
        rect,
        Rounding::same(10.0),
        Stroke::new(1.0_f32, p.border),
    );
    let inner = rect.shrink2(Vec2::new(16.0, 12.0));
    let mut content = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::left_to_right(Align::Center)),
    );
    let (avatar, _) = content.allocate_exact_size(Vec2::splat(34.0), Sense::hover());
    content
        .painter()
        .rect_filled(avatar, Rounding::same(8.0), p.accent_soft);
    content.painter().text(
        avatar.center(),
        Align2::CENTER_CENTER,
        widgets::initials(&saved.name),
        FontId::proportional(12.5),
        p.accent,
    );
    content.add_space(2.0);
    let text_w = (content.available_width() - 160.0).max(110.0);
    let _ = content.allocate_ui_with_layout(
        Vec2::new(text_w, 38.0),
        Layout::top_down(Align::LEFT),
        |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.add(
                egui::Label::new(
                    RichText::new(widgets::truncate(&saved.name, 40))
                        .size(13.5)
                        .strong()
                        .color(p.text),
                )
                .truncate(),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(widgets::truncate(&saved.target(), 64))
                        .size(10.8)
                        .color(p.faint),
                )
                .truncate(),
            );
        },
    );
    content.with_layout(Layout::right_to_left(Align::Center), |ui| {
        let open = egui::Button::new(
            RichText::new("Connect").size(12.0).color(Color32::WHITE),
        )
        .fill(p.accent)
        .rounding(Rounding::same(6.0))
        .min_size(Vec2::new(84.0, 27.0));
        if ui.add(open).clicked() {
            connect = true;
        }
        let driver = saved.driver_kind();
        widgets::chip(
            ui,
            &p,
            driver.label(),
            widgets::driver_color(&p, driver),
        );
    });
    resp.context_menu(|ui| {
        ui.set_min_width(170.0);
        if ui.button("Edit…").clicked() {
            action = Some(ConnectionMenuAction::Edit(saved.clone()));
            widgets::close_context_menu(ui);
        }
        if ui.button("Duplicate").clicked() {
            action = Some(ConnectionMenuAction::Duplicate(saved.clone()));
            widgets::close_context_menu(ui);
        }
        if ui.button("Export…").clicked() {
            action = Some(ConnectionMenuAction::Export(saved.clone()));
            widgets::close_context_menu(ui);
        }
        ui.separator();
        if ui.button("Delete…").clicked() {
            action = Some(ConnectionMenuAction::Delete(saved.clone()));
            widgets::close_context_menu(ui);
        }
    });
    (connect, action)
}

/// One group header in the connections list. Right-click offers rename/delete.
fn connection_group_header(
    ui: &mut Ui,
    p: &Palette,
    group: &ConnectionGroup,
    count: usize,
) -> Option<GroupMenuAction> {
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, Rounding::same(6.0), p.row_hover);
    }
    let icon_rect = Rect::from_center_size(
        egui::pos2(rect.left() + 12.0, rect.center().y),
        Vec2::new(14.0, 12.0),
    );
    widgets::draw_folder_icon(ui.painter(), icon_rect, p.amber);
    ui.painter().text(
        egui::pos2(rect.left() + 24.0, rect.center().y),
        Align2::LEFT_CENTER,
        widgets::truncate(&group.name, 34),
        FontId::proportional(12.0),
        p.text,
    );
    ui.painter().text(
        egui::pos2(rect.right() - 6.0, rect.center().y),
        Align2::RIGHT_CENTER,
        count.to_string(),
        FontId::proportional(10.0),
        p.faint,
    );
    let mut action = None;
    resp.context_menu(|ui| {
        ui.set_min_width(160.0);
        if ui.button("Rename group…").clicked() {
            action = Some(GroupMenuAction::Rename);
            widgets::close_context_menu(ui);
        }
        if ui.button("Delete group…").clicked() {
            action = Some(GroupMenuAction::Delete);
            widgets::close_context_menu(ui);
        }
    });
    action
}

    pub(crate) fn welcome_ui(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let mut connect: Option<SavedConnection> = None;
        let mut new_conn = false;
        let mut sample = false;
        let mut import_connections = false;
        let mut new_group = false;
        let mut group_rename: Option<(u64, String)> = None;
        let mut group_delete: Option<(u64, String)> = None;
        let mut connection_action: Option<ConnectionMenuAction> = None;
        // Taken out of `self` so the search field can be edited while the
        // connection list is borrowed for rendering.
        let mut connection_search = std::mem::take(&mut self.connection_search);

        ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(42.0);
                ui.vertical_centered(|ui| {
                    widgets::draw_logo(ui, 54.0, &p);
                    ui.add_space(14.0);
                    ui.label(
                        RichText::new("Database connections")
                            .size(24.0)
                            .strong()
                            .color(p.text),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(
                            "Pick up where you left off, or connect to SQLite, MySQL or PostgreSQL.",
                        )
                        .size(12.8)
                        .color(p.muted),
                    );
                    ui.add_space(24.0);

                    ui.scope(|ui| {
                        ui.set_width(620.0_f32.min(ui.available_width()));
                        if let Some(err) = &self.welcome_error {
                            egui::Frame::none()
                                .fill(if p.dark {
                                    Color32::from_rgb(58, 30, 31)
                                } else {
                                    Color32::from_rgb(254, 243, 242)
                                })
                                .stroke(Stroke::new(1.0_f32, p.red))
                                .rounding(Rounding::same(8.0))
                                .inner_margin(Margin::symmetric(12.0, 10.0))
                                .show(ui, |ui| {
                                    ui.label(RichText::new(err).size(11.8).color(p.red));
                                });
                            ui.add_space(10.0);
                        }

                        if self.store.connections.is_empty() && self.store.groups.is_empty() {
                            egui::Frame::none()
                                .fill(p.card)
                                .stroke(Stroke::new(1.0_f32, p.border))
                                .rounding(Rounding::same(10.0))
                                .inner_margin(Margin::symmetric(18.0, 20.0))
                                .show(ui, |ui| {
                                    ui.vertical_centered(|ui| {
                                        ui.label(
                                            RichText::new("No connections yet")
                                                .size(13.5)
                                                .strong()
                                                .color(p.text),
                                        );
                                        ui.add_space(4.0);
                                        ui.label(
                                            RichText::new(
                                                "Add a SQLite, MySQL or PostgreSQL connection, or start with the sample shop database.",
                                            )
                                            .size(11.5)
                                            .color(p.muted),
                                        );
                                    });
                                });
                            ui.add_space(10.0);
                        } else {
                            let search_frame = egui::Frame::none()
                                .fill(if p.dark {
                                    Color32::from_rgb(30, 32, 38)
                                } else {
                                    Color32::from_rgb(233, 236, 241)
                                })
                                .stroke(Stroke::new(1.0_f32, p.border))
                                .rounding(Rounding::same(7.0))
                                .inner_margin(Margin::symmetric(10.0, 7.0));
                            search_frame.show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let icon_rect = egui::Rect::from_min_size(
                                        ui.cursor().min,
                                        Vec2::new(18.0, 16.0),
                                    );
                                    widgets::draw_search_icon(ui.painter(), icon_rect, p.faint);
                                    ui.add_space(13.0);
                                    ui.add(
                                        TextEdit::singleline(&mut connection_search)
                                            .frame(false)
                                            .hint_text("Search connections…")
                                            .desired_width(f32::INFINITY),
                                    );
                                });
                            });
                            ui.add_space(10.0);

                            // `sub_1003f6d24` (0x1003f6d24): trim whitespace
                            // and lowercase the field before matching.
                            let query = connection_search.trim().to_lowercase();
                            let filtered: Vec<SavedConnection> = self
                                .store
                                .connections
                                .iter()
                                .filter(|saved| {
                                    query.is_empty()
                                        || saved.name.to_lowercase().contains(&query)
                                        || saved.target().to_lowercase().contains(&query)
                                        || saved
                                            .driver_kind()
                                            .label()
                                            .to_lowercase()
                                            .contains(&query)
                                })
                                .cloned()
                                .collect();
                            let groups = self.store.groups.clone();
                            let known_groups: std::collections::HashSet<u64> =
                                groups.iter().map(|group| group.id).collect();
                            let ungrouped: Vec<&SavedConnection> = filtered
                                .iter()
                                .filter(|saved| {
                                    saved
                                        .group_id
                                        .map(|id| !known_groups.contains(&id))
                                        .unwrap_or(true)
                                })
                                .collect();
                            let has_groups = !groups.is_empty();

                            if filtered.is_empty() && !query.is_empty() {
                                egui::Frame::none()
                                    .fill(p.card)
                                    .stroke(Stroke::new(1.0_f32, p.border))
                                    .rounding(Rounding::same(10.0))
                                    .inner_margin(Margin::symmetric(18.0, 18.0))
                                    .show(ui, |ui| {
                                        ui.vertical_centered(|ui| {
                                            ui.label(
                                                RichText::new(format!(
                                                    "No connections match “{}”",
                                                    connection_search.trim()
                                                ))
                                                .size(12.5)
                                                .color(p.muted),
                                            );
                                            ui.add_space(8.0);
                                            let clear = egui::Button::new(
                                                RichText::new("Clear search")
                                                    .size(11.0)
                                                    .color(p.text),
                                            )
                                            .fill(p.card)
                                            .stroke(Stroke::new(1.0_f32, p.border))
                                            .rounding(Rounding::same(6.0))
                                            .min_size(Vec2::new(110.0, 26.0));
                                            if ui.add(clear).clicked() {
                                                connection_search.clear();
                                            }
                                        });
                                    });
                                ui.add_space(10.0);
                            } else {
                                if has_groups && !ungrouped.is_empty() {
                                    ui.label(
                                        RichText::new("UNGROUPED")
                                            .size(9.5)
                                            .color(p.faint),
                                    );
                                    ui.add_space(2.0);
                                }
                                for saved in &ungrouped {
                                    let (do_connect, action) =
                                        Self::connection_card(ui, &p, saved);
                                    if do_connect {
                                        connect = Some((*saved).clone());
                                    }
                                    if action.is_some() {
                                        connection_action = action;
                                    }
                                    ui.add_space(8.0);
                                }
                                for group in &groups {
                                    let items: Vec<&SavedConnection> = filtered
                                        .iter()
                                        .filter(|saved| saved.group_id == Some(group.id))
                                        .collect();
                                    if items.is_empty() && !query.is_empty() {
                                        continue;
                                    }
                                    ui.add_space(6.0);
                                    match Self::connection_group_header(
                                        ui, &p, group, items.len(),
                                    ) {
                                        Some(GroupMenuAction::Rename) => {
                                            group_rename =
                                                Some((group.id, group.name.clone()));
                                        }
                                        Some(GroupMenuAction::Delete) => {
                                            group_delete =
                                                Some((group.id, group.name.clone()));
                                        }
                                        None => {}
                                    }
                                    for saved in items {
                                        let (do_connect, action) =
                                            Self::connection_card(ui, &p, saved);
                                        if do_connect {
                                            connect = Some((*saved).clone());
                                        }
                                        if action.is_some() {
                                            connection_action = action;
                                        }
                                        ui.add_space(8.0);
                                    }
                                }
                            }
                        }

                        ui.add_space(6.0);
                        ui.horizontal_wrapped(|ui| {
                            let new = egui::Button::new(
                                RichText::new("+   New connection")
                                    .size(12.2)
                                    .color(Color32::WHITE),
                            )
                            .fill(p.accent)
                            .rounding(Rounding::same(7.0))
                            .min_size(Vec2::new(180.0, 30.0));
                            if ui.add(new).clicked() {
                                new_conn = true;
                            }
                            let group_btn = egui::Button::new(
                                RichText::new("+   New group").size(12.0).color(p.text),
                            )
                            .fill(p.card)
                            .stroke(Stroke::new(1.0_f32, p.border))
                            .rounding(Rounding::same(7.0))
                            .min_size(Vec2::new(120.0, 30.0));
                            if ui.add(group_btn).clicked() {
                                new_group = true;
                            }
                            let import_btn = egui::Button::new(
                                RichText::new("Import connections").size(12.0).color(p.text),
                            )
                            .fill(p.card)
                            .stroke(Stroke::new(1.0_f32, p.border))
                            .rounding(Rounding::same(7.0))
                            .min_size(Vec2::new(160.0, 30.0));
                            if ui.add(import_btn).clicked() {
                                import_connections = true;
                            }
                            let sample_btn = egui::Button::new(
                                RichText::new("Open sample database").size(12.0).color(p.text),
                            )
                            .fill(p.card)
                            .stroke(Stroke::new(1.0_f32, p.border))
                            .rounding(Rounding::same(7.0))
                            .min_size(Vec2::new(170.0, 30.0));
                            if ui.add(sample_btn).clicked() {
                                sample = true;
                            }
                        });
                        ui.add_space(12.0);
                        ui.label(
                            RichText::new(
                                "SQLite files, MySQL servers and PostgreSQL servers are all supported.",
                            )
                            .size(10.5)
                            .color(p.faint),
                        );
                    });
                });
            });

        self.connection_search = connection_search;

        if let Some(saved) = connect {
            if let Err(e) = self.connect_saved(saved) {
                self.welcome_error = Some(e);
            }
        }
        if new_conn {
            self.open_new_connection_dialog();
        }
        if sample {
            self.open_sample();
        }
        if import_connections {
            self.import_connections();
        }
        if new_group {
            self.open_group_editor(None);
        }
        if let Some((id, name)) = group_rename {
            self.open_group_editor(Some((id, name)));
        }
        if let Some((id, name)) = group_delete {
            self.group_delete = Some(crate::app::GroupDelete { id, name });
        }
        if let Some(action) = connection_action {
            match action {
                ConnectionMenuAction::Edit(saved) => {
                    self.open_edit_connection_dialog(&saved);
                }
                ConnectionMenuAction::Duplicate(saved) => {
                    self.duplicate_connection(&saved);
                }
                ConnectionMenuAction::Delete(saved) => {
                    self.confirm_delete_connection(&saved);
                }
                ConnectionMenuAction::Export(saved) => {
                    self.export_connection(&saved);
                }
            }
        }
    }

    pub(crate) fn no_tabs_ui(&mut self, ui: &mut Ui) {
        let p = self.palette;
        let mut new_query = false;
        ui.vertical_centered(|ui| {
            ui.add_space(120.0);
            widgets::draw_logo(ui, 40.0, &p);
            ui.add_space(12.0);
            ui.label(
                RichText::new("No tabs open")
                    .size(18.0)
                    .strong()
                    .color(p.text),
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new("Open a query or pick a table from the sidebar.")
                    .size(11.8)
                    .color(p.muted),
            );
            ui.add_space(14.0);
            if ui
                .add(
                    egui::Button::new(
                        RichText::new("+   New query")
                            .size(12.0)
                            .color(Color32::WHITE),
                    )
                    .fill(p.accent)
                    .rounding(Rounding::same(6.0))
                    .min_size(Vec2::new(130.0, 29.0)),
                )
                .clicked()
            {
                new_query = true;
            }
        });
        if new_query {
            self.new_query_tab(true);
        }
    }

    pub(crate) fn open_new_connection_dialog(&mut self) {
        if self.new_connection.is_none() {
            let name = "My database".to_string();
            let path = default_new_path(&name);
            self.new_connection = Some(NewConnection {
                driver: Driver::Sqlite,
                name,
                path,
                path_touched: false,
                host: "127.0.0.1".to_string(),
                port: String::new(),
                user: String::new(),
                password: String::new(),
                database: String::new(),
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
            });
        }
    }

    pub(crate) fn new_connection_dialog(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let Some(mut form) = self.new_connection.take() else {
            return;
        };
        let groups = self.store.groups.clone();
        let mut close = false;
        let mut submit = false;
        let mut open_existing = false;

        egui::Window::new(if form.editing.is_some() {
            "Edit connection"
        } else {
            "New connection"
        })
        .collapsible(false)
        .resizable(false)
        .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
        .frame(
            egui::Frame::window(&ctx.style())
                .fill(p.panel)
                .stroke(Stroke::new(1.0_f32, p.border))
                .rounding(Rounding::same(10.0))
                .inner_margin(Margin::symmetric(18.0, 16.0)),
        )
        .show(ctx, |ui| {
            ui.set_width(462.0);
            ui.label(
                RichText::new(if form.editing.is_some() {
                    "Edit connection"
                } else {
                    "Connect a database"
                })
                .size(16.0)
                .strong()
                .color(p.text),
            );
            ui.add_space(3.0);
            ui.label(
                RichText::new(if form.editing.is_some() {
                    "Update this saved connection."
                } else {
                    "Choose an engine, then fill in where it lives."
                })
                .size(11.4)
                .color(p.muted),
            );
            ui.add_space(12.0);

            egui::ScrollArea::vertical()
                .id_salt("connection-form")
                .max_height((ctx.screen_rect().height() - 360.0).clamp(300.0, 520.0))
                .show(ui, |ui| {
            let previous_driver = form.driver;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                for driver in Driver::all() {
                    let selected = form.driver == driver;
                    if widgets::segmented(ui, &p, driver.label(), selected).clicked() {
                        form.driver = driver;
                    }
                }
            });
            if form.driver != previous_driver {
                if form.host.trim().is_empty() {
                    form.host = "127.0.0.1".to_string();
                }
                let previous_default = previous_driver.default_port().to_string();
                if form.port.trim().is_empty() || form.port.trim() == previous_default {
                    form.port = match form.driver {
                        Driver::Sqlite => String::new(),
                        driver => driver.default_port().to_string(),
                    };
                }
                let previous_user = match previous_driver {
                    Driver::MySql => "root",
                    Driver::Postgres => "postgres",
                    Driver::Sqlite => "",
                };
                if form.user.trim().is_empty() || form.user.trim() == previous_user {
                    form.user = match form.driver {
                        Driver::MySql => "root".to_string(),
                        Driver::Postgres => "postgres".to_string(),
                        Driver::Sqlite => String::new(),
                    };
                }
                form.error = None;
            }

            ui.add_space(12.0);
            ui.label(RichText::new("Name").size(11.0).color(p.muted));
            ui.add_space(3.0);
            let name_resp = ui.add_sized(
                Vec2::new(ui.available_width(), 27.0),
                TextEdit::singleline(&mut form.name),
            );
            if name_resp.changed() && form.driver == Driver::Sqlite && !form.path_touched {
                form.path = default_new_path(&form.name);
            }

            ui.add_space(10.0);
            ui.label(RichText::new("Group").size(11.0).color(p.muted));
            ui.add_space(3.0);
            let group_label = form
                .group_id
                .and_then(|id| {
                    groups
                        .iter()
                        .find(|group| group.id == id)
                        .map(|group| group.name.clone())
                })
                .unwrap_or_else(|| "No group".to_string());
            egui::ComboBox::from_id_salt("connection-group")
                .selected_text(group_label)
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(form.group_id.is_none(), "No group")
                        .clicked()
                    {
                        form.group_id = None;
                    }
                    for group in &groups {
                        if ui
                            .selectable_label(
                                form.group_id == Some(group.id),
                                &group.name,
                            )
                            .clicked()
                        {
                            form.group_id = Some(group.id);
                        }
                    }
                });
            ui.add_space(10.0);
            if ui
                .checkbox(
                    &mut form.read_only,
                    RichText::new("Open read-only (block writes)")
                        .size(11.4)
                        .color(p.text),
                )
                .changed()
            {
                form.error = None;
            }
            ui.add_space(10.0);

            if form.driver == Driver::Sqlite {
                ui.add_space(10.0);
                ui.label(RichText::new("File path").size(11.0).color(p.muted));
                ui.add_space(3.0);
                ui.horizontal(|ui| {
                    let path_resp = ui.add_sized(
                        Vec2::new(ui.available_width() - 100.0, 27.0),
                        TextEdit::singleline(&mut form.path),
                    );
                    if path_resp.changed() {
                        form.path_touched = true;
                    }
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Documents").size(11.0))
                                .fill(p.card)
                                .stroke(Stroke::new(1.0_f32, p.border)),
                        )
                        .clicked()
                    {
                        form.path = crate::app::default_new_path(&form.name);
                        form.path_touched = true;
                    }
                });
            } else {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Host").size(11.0).color(p.muted));
                        ui.add_space(3.0);
                        ui.add_sized(Vec2::new(258.0, 27.0), TextEdit::singleline(&mut form.host));
                    });
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Port").size(11.0).color(p.muted));
                        ui.add_space(3.0);
                        ui.add_sized(Vec2::new(172.0, 27.0), TextEdit::singleline(&mut form.port));
                    });
                });
                ui.add_space(10.0);
                ui.label(RichText::new("User").size(11.0).color(p.muted));
                ui.add_space(3.0);
                ui.add_sized(
                    Vec2::new(ui.available_width(), 27.0),
                    TextEdit::singleline(&mut form.user),
                );
                ui.add_space(10.0);
                ui.label(RichText::new("Password").size(11.0).color(p.muted));
                ui.add_space(3.0);
                ui.add_sized(
                    Vec2::new(ui.available_width(), 27.0),
                    TextEdit::singleline(&mut form.password).password(true),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(if form.editing.is_some() {
                        "Saved in the Keychain. Leave blank to keep the current password."
                    } else {
                        "Stored in the macOS Keychain when available."
                    })
                    .size(10.0)
                    .color(p.faint),
                );
                ui.add_space(10.0);
                ui.label(RichText::new("Database").size(11.0).color(p.muted));
                ui.add_space(3.0);
                ui.add_sized(
                    Vec2::new(ui.available_width(), 27.0),
                    TextEdit::singleline(&mut form.database),
                );
                ui.add_space(12.0);
                let advanced_open = form.ssl_mode != "disable" || form.ssh_enabled;
                egui::CollapsingHeader::new(RichText::new("TLS / SSH tunnel").size(11.4))
                    .default_open(advanced_open)
                    .show(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("SSL").size(11.0).color(p.muted));
                    ui.add_space(4.0);
                    egui::ComboBox::from_id_salt("connection-ssl")
                        .selected_text(ssl_mode_label(&form.ssl_mode))
                        .width(250.0)
                        .show_ui(ui, |ui| {
                            for (value, label) in SSL_MODE_OPTIONS {
                                ui.selectable_value(
                                    &mut form.ssl_mode,
                                    value.to_string(),
                                    *label,
                                );
                            }
                        });
                });
                if form.ssl_mode != "disable" {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("Root certificate (optional, PEM or DER)")
                            .size(11.0)
                            .color(p.muted),
                    );
                    ui.add_space(3.0);
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            Vec2::new(ui.available_width() - 86.0, 27.0),
                            TextEdit::singleline(&mut form.ssl_root_cert),
                        );
                        if ui
                            .add(
                                egui::Button::new(RichText::new("Browse…").size(11.0))
                                    .fill(p.card)
                                    .stroke(Stroke::new(1.0_f32, p.border)),
                            )
                            .clicked()
                        {
                            if let Some(path) = crate::app::pick_certificate_file() {
                                form.ssl_root_cert = path.to_string_lossy().to_string();
                            }
                        }
                    });
                }
                ui.add_space(12.0);
                ui.checkbox(
                    &mut form.ssh_enabled,
                    RichText::new("Connect through an SSH tunnel")
                        .size(11.4)
                        .color(p.text),
                );
                if form.ssh_enabled {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new("SSH host").size(11.0).color(p.muted));
                            ui.add_space(3.0);
                            ui.add_sized(
                                Vec2::new(258.0, 27.0),
                                TextEdit::singleline(&mut form.ssh_host),
                            );
                        });
                        ui.add_space(8.0);
                        ui.vertical(|ui| {
                            ui.label(RichText::new("SSH port").size(11.0).color(p.muted));
                            ui.add_space(3.0);
                            ui.add_sized(
                                Vec2::new(172.0, 27.0),
                                TextEdit::singleline(&mut form.ssh_port),
                            );
                        });
                    });
                    ui.add_space(8.0);
                    ui.label(RichText::new("SSH user").size(11.0).color(p.muted));
                    ui.add_space(3.0);
                    ui.add_sized(
                        Vec2::new(ui.available_width(), 27.0),
                        TextEdit::singleline(&mut form.ssh_user),
                    );
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("Private key (optional; the ssh agent is used otherwise)")
                            .size(11.0)
                            .color(p.muted),
                    );
                    ui.add_space(3.0);
                    ui.horizontal(|ui| {
                        ui.add_sized(
                            Vec2::new(ui.available_width() - 86.0, 27.0),
                            TextEdit::singleline(&mut form.ssh_key_path),
                        );
                        if ui
                            .add(
                                egui::Button::new(RichText::new("Browse…").size(11.0))
                                    .fill(p.card)
                                    .stroke(Stroke::new(1.0_f32, p.border)),
                            )
                            .clicked()
                        {
                            if let Some(path) = crate::app::pick_ssh_key_file() {
                                form.ssh_key_path = path.to_string_lossy().to_string();
                            }
                        }
                    });
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(
                            "ssh runs with BatchMode=yes: use an agent or key. Host keys are accepted on first use.",
                        )
                        .size(10.0)
                        .color(p.faint),
                    );
                }
                });
            }

            });
            if let Some(err) = &form.error {
                ui.add_space(8.0);
                ui.label(RichText::new(err).size(11.2).color(p.red));
            }

            ui.add_space(16.0);
            ui.horizontal(|ui| {
                let primary_label = if form.editing.is_some() {
                    "Save"
                } else if form.driver == Driver::Sqlite {
                    "Create & connect"
                } else {
                    "Connect"
                };
                let primary = egui::Button::new(
                    RichText::new(primary_label)
                        .size(12.0)
                        .color(Color32::WHITE),
                )
                .fill(p.accent)
                .rounding(Rounding::same(6.0))
                .min_size(Vec2::new(135.0, 29.0));
                if ui.add(primary).clicked() {
                    submit = true;
                }
                if form.driver == Driver::Sqlite && form.editing.is_none() {
                    let open_btn =
                        egui::Button::new(RichText::new("Open existing").size(12.0).color(p.text))
                            .fill(p.card)
                            .stroke(Stroke::new(1.0_f32, p.border))
                            .rounding(Rounding::same(6.0))
                            .min_size(Vec2::new(120.0, 29.0));
                    if ui.add(open_btn).clicked() {
                        open_existing = true;
                    }
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(RichText::new("Cancel").size(12.0).color(p.muted))
                                .fill(Color32::TRANSPARENT)
                                .stroke(Stroke::NONE),
                        )
                        .clicked()
                    {
                        close = true;
                    }
                });
            });
            ui.add_space(8.0);
            ui.label(
                RichText::new(match form.driver {
                    Driver::Sqlite => {
                        "Careful: connecting to an existing file opens it read-write."
                    }
                    Driver::MySql => "MySQL servers are contacted over the network.",
                    Driver::Postgres => "PostgreSQL servers are contacted over the network.",
                })
                .size(10.2)
                .color(p.faint),
            );
        });

        if submit || open_existing {
            if submit {
                if let Some(id) = form.editing {
                    if let Err(e) = validate_new_form(&form, false) {
                        form.error = Some(e);
                        self.new_connection = Some(form);
                        return;
                    }
                    let driver = form.driver;
                    let saved = SavedConnection {
                        id,
                        name: form.name.trim().to_string(),
                        path: if driver == Driver::Sqlite {
                            form.path.trim().to_string()
                        } else {
                            String::new()
                        },
                        driver: driver.label().to_string(),
                        host: if driver == Driver::Sqlite {
                            String::new()
                        } else {
                            form.host.trim().to_string()
                        },
                        port: if driver == Driver::Sqlite {
                            0
                        } else {
                            form.port
                                .trim()
                                .parse::<u16>()
                                .unwrap_or(driver.default_port())
                        },
                        user: if driver == Driver::Sqlite {
                            String::new()
                        } else {
                            form.user.trim().to_string()
                        },
                        password: if driver == Driver::Sqlite {
                            String::new()
                        } else {
                            form.password.clone()
                        },
                        database: if driver == Driver::Sqlite {
                            String::new()
                        } else {
                            form.database.trim().to_string()
                        },
                        read_only: form.read_only,
                        ssl_mode: if driver == Driver::Sqlite || form.ssl_mode.trim().is_empty() {
                            "disable".to_string()
                        } else {
                            form.ssl_mode.clone()
                        },
                        ssl_root_cert: if driver == Driver::Sqlite
                            || form.ssl_mode.trim() == "disable"
                        {
                            String::new()
                        } else {
                            form.ssl_root_cert.trim().to_string()
                        },
                        ssh_host: if driver == Driver::Sqlite || !form.ssh_enabled {
                            String::new()
                        } else {
                            form.ssh_host.trim().to_string()
                        },
                        ssh_port: if driver == Driver::Sqlite || !form.ssh_enabled {
                            0
                        } else {
                            form.ssh_port.trim().parse::<u16>().unwrap_or(22)
                        },
                        ssh_user: if driver == Driver::Sqlite || !form.ssh_enabled {
                            String::new()
                        } else {
                            form.ssh_user.trim().to_string()
                        },
                        ssh_key_path: if driver == Driver::Sqlite || !form.ssh_enabled {
                            String::new()
                        } else {
                            form.ssh_key_path.trim().to_string()
                        },
                        group_id: form.group_id,
                    };
                    self.store.update_connection(saved.clone());
                    let mut display = saved;
                    display.password.clear();
                    if let Some(live) = &mut self.live {
                        if live.saved.id == display.id {
                            live.db.set_read_only(display.read_only);
                            live.saved = display;
                        }
                    }
                    self.welcome_error = None;
                    self.push_toast("Connection updated".to_string(), ToastKind::Info);
                    return;
                }
            }
            let create = form.driver == Driver::Sqlite && submit;
            if let Err(e) = validate_new_form(&form, create) {
                form.error = Some(e);
                self.new_connection = Some(form);
                return;
            }
            let mut saved = match form.driver {
                Driver::Sqlite => {
                    let path_text = form.path.trim().to_string();
                    if let Some(existing) = self
                        .store
                        .connections
                        .iter()
                        .find(|c| c.driver_kind() == Driver::Sqlite && c.path == path_text)
                        .cloned()
                    {
                        existing
                    } else {
                        self.store.add(form.name.trim(), &path_text)
                    }
                }
                driver => {
                    let host = form.host.trim().to_string();
                    let port = form
                        .port
                        .trim()
                        .parse::<u16>()
                        .unwrap_or(driver.default_port());
                    let user = form.user.trim().to_string();
                    let database = form.database.trim().to_string();
                    let ssl_mode = if form.ssl_mode.trim().is_empty() {
                        "disable".to_string()
                    } else {
                        form.ssl_mode.clone()
                    };
                    let ssl_root_cert = if ssl_mode == "disable" {
                        String::new()
                    } else {
                        form.ssl_root_cert.trim().to_string()
                    };
                    let ssh_host = if form.ssh_enabled {
                        form.ssh_host.trim().to_string()
                    } else {
                        String::new()
                    };
                    let ssh_port = if form.ssh_enabled {
                        form.ssh_port.trim().parse::<u16>().unwrap_or(22)
                    } else {
                        0
                    };
                    let ssh_user = if form.ssh_enabled {
                        form.ssh_user.trim().to_string()
                    } else {
                        String::new()
                    };
                    let ssh_key_path = if form.ssh_enabled {
                        form.ssh_key_path.trim().to_string()
                    } else {
                        String::new()
                    };
                    let existing = self
                        .store
                        .connections
                        .iter()
                        .find(|c| {
                            c.driver_kind() == driver
                                && c.host == host
                                && c.port == port
                                && c.user == user
                                && c.database == database
                                && c.ssl_mode == ssl_mode
                                && c.ssl_root_cert == ssl_root_cert
                                && c.ssh_host == ssh_host
                                && c.ssh_port == ssh_port
                                && c.ssh_user == ssh_user
                                && c.ssh_key_path == ssh_key_path
                        })
                        .cloned();
                    if let Some(mut existing) = existing {
                        // Reusing a saved target still honors a password typed
                        // into this form; the Keychain copy is updated.
                        if !form.password.is_empty() {
                            existing.password = form.password.clone();
                            self.store.update_connection(existing.clone());
                        }
                        existing
                    } else {
                        self.store.add_connection(SavedConnection {
                            id: 0,
                            name: form.name.trim().to_string(),
                            path: String::new(),
                            driver: driver.label().to_string(),
                            host,
                            port,
                            user,
                            password: form.password.clone(),
                            database,
                            read_only: form.read_only,
                            ssl_mode,
                            ssl_root_cert,
                            ssh_host,
                            ssh_port,
                            ssh_user,
                            ssh_key_path,
                            group_id: form.group_id,
                        })
                    }
                }
            };
            if saved.group_id != form.group_id {
                self.store.set_connection_group(saved.id, form.group_id);
                saved.group_id = form.group_id;
            }
            match self.connect_saved(saved) {
                Ok(()) => {}
                Err(e) => {
                    form.error = Some(e);
                    self.new_connection = Some(form);
                }
            }
        } else if !close {
            self.new_connection = Some(form);
        }
    }

    pub(crate) fn db_tools_window(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        let Some(mut tools) = self.db_tools.take() else {
            return;
        };
        let is_sqlite = self
            .live
            .as_ref()
            .map(|live| live.db.driver == Driver::Sqlite)
            .unwrap_or(false);
        let mut close = false;
        let mut do_export = false;
        let mut do_import = false;
        let mut do_import_data = false;
        let mut do_backup = false;
        let mut delete_backup: Option<PathBuf> = None;
        let mut restore_backup: Option<PathBuf> = None;
        let mut reveal: Option<PathBuf> = None;

        let primary = |label: &str| {
            egui::Button::new(RichText::new(label).size(12.0).color(Color32::WHITE))
                .fill(p.accent)
                .rounding(Rounding::same(6.0))
                .min_size(Vec2::new(126.0, 28.0))
        };
        let secondary = |label: &str| {
            egui::Button::new(RichText::new(label).size(11.0).color(p.text))
                .fill(p.card)
                .stroke(Stroke::new(1.0_f32, p.border))
                .rounding(Rounding::same(6.0))
        };

        egui::Window::new("Database tools")
            .collapsible(false)
            .resizable(false)
            .default_width(600.0)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(p.panel)
                    .stroke(Stroke::new(1.0_f32, p.border))
                    .rounding(Rounding::same(10.0))
                    .inner_margin(Margin::symmetric(18.0, 16.0)),
            )
            .show(ctx, |ui| {
                ui.set_width(600.0);
                ui.horizontal(|ui| {
                    for (tab, label) in [
                        (ToolsTab::Export, "Export SQL"),
                        (ToolsTab::Import, "Import SQL"),
                        (ToolsTab::Backups, "Backups"),
                    ] {
                        if widgets::segmented(ui, &p, label, tools.tab == tab).clicked() {
                            tools.tab = tab;
                            tools.confirm_restore = None;
                        }
                    }
                });
                ui.add_space(12.0);

                match tools.tab {
                    ToolsTab::Export => {
                        ui.label(RichText::new("Scope").size(10.4).color(p.faint));
                        ui.horizontal(|ui| {
                            if widgets::segmented(ui, &p, "Whole database", !tools.scope_table)
                                .clicked()
                            {
                                tools.scope_table = false;
                            }
                            match &tools.table {
                                Some(table) => {
                                    let label = format!("Table: {table}");
                                    if widgets::segmented(ui, &p, &label, tools.scope_table)
                                        .clicked()
                                    {
                                        tools.scope_table = true;
                                    }
                                }
                                None => {
                                    tools.scope_table = false;
                                }
                            }
                        });
                        if tools.scope_table {
                            if let Some(filter) = &tools.filter {
                                ui.add_space(6.0);
                                ui.checkbox(
                                    &mut tools.use_filter,
                                    RichText::new("Only rows matching the active filter")
                                        .size(11.0)
                                        .color(p.text),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "{} filter clause{}",
                                        filter.active_count(),
                                        if filter.active_count() == 1 { "" } else { "s" }
                                    ))
                                    .size(10.2)
                                    .color(p.faint),
                                );
                            }
                        }
                        ui.add_space(10.0);
                        ui.label(RichText::new("SQL file").size(10.4).color(p.faint));
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                Vec2::new((ui.available_width() - 96.0).max(160.0), 25.0),
                                TextEdit::singleline(&mut tools.export_path)
                                    .hint_text("~/Desktop/dump.sql"),
                            );
                            if ui.add(secondary("Save as…")).clicked() {
                                if let Some(path) = crate::app::save_sql_file(&tools.export_path) {
                                    tools.export_path = path.display().to_string();
                                }
                            }
                        });
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(
                                "CREATE statements plus one INSERT per row. Write access to the path is required.",
                            )
                            .size(10.4)
                            .color(p.faint),
                        );
                        ui.add_space(12.0);
                        if ui.add(primary("Export SQL")).clicked() {
                            do_export = true;
                        }
                    }
                    ToolsTab::Import => {
                        ui.label(
                            RichText::new("Structured data (CSV, Excel, JSON)")
                                .size(10.4)
                                .color(p.faint),
                        );
                        ui.add_space(6.0);
                        if ui.add(primary("Import data…")).clicked() {
                            do_import_data = true;
                        }
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(
                                "Map columns, preview the rows and append to a table or create a new one.",
                            )
                            .size(10.4)
                            .color(p.faint),
                        );
                        ui.add_space(14.0);
                        ui.separator();
                        ui.add_space(12.0);
                        ui.label(RichText::new("SQL file to run").size(10.4).color(p.faint));
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                Vec2::new((ui.available_width() - 96.0).max(160.0), 25.0),
                                TextEdit::singleline(&mut tools.import_path)
                                    .hint_text("~/Downloads/dump.sql"),
                            );
                            if ui.add(secondary("Browse…")).clicked() {
                                let start = crate::app::browse_start_dir(&tools.import_path);
                                if let Some(path) = crate::app::pick_sql_file(start) {
                                    tools.import_path = path.display().to_string();
                                }
                            }
                        });
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(
                                "Every statement in the file runs against the current connection. This can drop or overwrite tables.",
                            )
                            .size(10.4)
                            .color(p.faint),
                        );
                        ui.add_space(12.0);
                        if ui.add(primary("Import SQL")).clicked() {
                            do_import = true;
                        }
                    }
                    ToolsTab::Backups => {
                        ui.horizontal(|ui| {
                            if ui.add(primary("Create backup now")).clicked() {
                                do_backup = true;
                            }
                            ui.label(
                                RichText::new(if is_sqlite {
                                    "SQLite: byte-for-byte copy of the database file."
                                } else {
                                    "Server engines: SQL dump of every table."
                                })
                                .size(10.4)
                                .color(p.faint),
                            );
                        });
                        ui.add_space(12.0);
                        if tools.backups.is_empty() {
                            ui.label(
                                RichText::new("No backups yet for this connection.")
                                    .size(11.0)
                                    .color(p.muted),
                            );
                        } else {
                            ScrollArea::vertical().max_height(230.0).show(ui, |ui| {
                                for backup in &tools.backups {
                                    ui.horizontal(|ui| {
                                        ui.vertical(|ui| {
                                            ui.label(
                                                RichText::new(&backup.name)
                                                    .size(11.4)
                                                    .color(p.text),
                                            );
                                            ui.label(
                                                RichText::new(format!(
                                                    "{} · {}",
                                                    fmt_bytes(backup.bytes),
                                                    fmt_backup_time(backup.modified)
                                                ))
                                                .size(10.2)
                                                .color(p.faint),
                                            );
                                        });
                                        ui.with_layout(
                                            Layout::right_to_left(Align::Center),
                                            |ui| {
                                                if ui.add(secondary("Delete")).clicked() {
                                                    delete_backup = Some(backup.path.clone());
                                                }
                                                if is_sqlite
                                                    && backup.name.ends_with(".db")
                                                    && ui.add(secondary("Restore")).clicked()
                                                {
                                                    tools.confirm_restore =
                                                        Some(backup.path.clone());
                                                }
                                                if ui.add(secondary("Show")).clicked() {
                                                    reveal = Some(backup.path.clone());
                                                }
                                            },
                                        );
                                    });
                                    ui.add_space(2.0);
                                    ui.separator();
                                }
                            });
                        }
                        if let Some(path) = tools.confirm_restore.clone() {
                            ui.add_space(8.0);
                            egui::Frame::none()
                                .fill(p.card)
                                .stroke(Stroke::new(1.0_f32, p.red.gamma_multiply(0.6)))
                                .rounding(Rounding::same(8.0))
                                .inner_margin(Margin::symmetric(12.0, 10.0))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(format!(
                                            "Restore {}?",
                                            path.file_name()
                                                .map(|n| n.to_string_lossy().to_string())
                                                .unwrap_or_else(|| "this backup".to_string())
                                        ))
                                        .size(11.8)
                                        .color(p.text),
                                    );
                                    ui.label(
                                        RichText::new(
                                            "The current database file is replaced and the connection reopens. This cannot be undone.",
                                        )
                                        .size(10.6)
                                        .color(p.muted),
                                    );
                                    ui.add_space(8.0);
                                    ui.horizontal(|ui| {
                                        if ui
                                            .add(
                                                egui::Button::new(
                                                    RichText::new("Restore now")
                                                        .size(11.5)
                                                        .color(Color32::WHITE),
                                                )
                                                .fill(p.red)
                                                .rounding(Rounding::same(6.0)),
                                            )
                                            .clicked()
                                        {
                                            restore_backup = Some(path.clone());
                                        }
                                        if ui.add(secondary("Cancel")).clicked() {
                                            tools.confirm_restore = None;
                                        }
                                    });
                                });
                        }
                    }
                }

                if let Some(message) = &tools.message {
                    ui.add_space(10.0);
                    let color = if tools.message_is_error { p.red } else { p.green };
                    ui.label(RichText::new(message).size(10.8).color(color));
                }

                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if ui.add(secondary("Close")).clicked() {
                        close = true;
                    }
                });
            });

        if do_export {
            let path = if tools.export_path.trim().is_empty() {
                db::default_dump_path(
                    &tools
                        .table
                        .as_ref()
                        .map(|table| table.display())
                        .unwrap_or_else(|| "dataloom".to_string()),
                )
            } else {
                expand_path(&tools.export_path)
            };
            let filter = if tools.use_filter {
                tools.filter.as_ref()
            } else {
                None
            };
            let result = match &self.live {
                None => Err("The connection is closed.".to_string()),
                Some(live) => {
                    let dump = if tools.scope_table {
                        match &tools.table {
                            Some(table) => match filter {
                                Some(filter) => live
                                    .db
                                    .dump_table_sql(&table.sql(), filter, &[]),
                                None => live.db.dump_table_sql(
                                    &table.sql(),
                                    &db::TableFilter::default(),
                                    &[],
                                ),
                            },
                            None => Err("Open a table tab to export a single table.".to_string()),
                        }
                    } else {
                        live.db.dump_database_sql(&live.saved.name)
                    };
                    dump.and_then(|sql| {
                        std::fs::write(&path, sql)
                            .map_err(|e| format!("Could not write {}: {e}", path.display()))
                    })
                }
            };
            match result {
                Ok(()) => {
                    tools.message = Some(format!("Exported to {}", path.display()));
                    tools.message_is_error = false;
                    tools.export_path = path.display().to_string();
                }
                Err(error) => {
                    tools.message = Some(error);
                    tools.message_is_error = true;
                }
            }
        }

        if do_import_data {
            if let Some(path) = crate::import::pick_data_file() {
                self.open_import_dialog(path);
            }
        }
        if do_import {
            let path = expand_path(&tools.import_path);
            let result = if tools.import_path.trim().is_empty() {
                Err("Choose an SQL file to import.".to_string())
            } else {
                match std::fs::read_to_string(&path) {
                    Err(e) => Err(format!("Could not read {}: {e}", path.display())),
                    Ok(script) => match &self.live {
                        None => Err("The connection is closed.".to_string()),
                        Some(live) => live.db.import_sql(&script),
                    },
                }
            };
            match result {
                Ok(count) => {
                    tools.message = Some(format!(
                        "Ran {count} statement{} from {}",
                        if count == 1 { "" } else { "s" },
                        path.display()
                    ));
                    tools.message_is_error = false;
                    self.refresh_after_import();
                }
                Err(error) => {
                    tools.message = Some(error);
                    tools.message_is_error = true;
                }
            }
        }

        if do_backup {
            let result = match &self.live {
                Some(live) => live.db.create_backup(&live.saved),
                None => Err("The connection is closed.".to_string()),
            };
            match result {
                Ok(path) => {
                    tools.message = Some(format!("Backup written to {}", path.display()));
                    tools.message_is_error = false;
                }
                Err(error) => {
                    tools.message = Some(error);
                    tools.message_is_error = true;
                }
            }
            tools.backups = self
                .live
                .as_ref()
                .map(|live| db::list_backups(&live.saved))
                .unwrap_or_default();
        }

        if let Some(path) = delete_backup {
            match db::delete_backup(&path) {
                Ok(()) => {
                    tools.message = Some(format!("Deleted {}", path.display()));
                    tools.message_is_error = false;
                }
                Err(error) => {
                    tools.message = Some(error);
                    tools.message_is_error = true;
                }
            }
            tools.confirm_restore = None;
            tools.backups = self
                .live
                .as_ref()
                .map(|live| db::list_backups(&live.saved))
                .unwrap_or_default();
        }

        if let Some(path) = reveal {
            let _ = std::process::Command::new("open")
                .arg("-R")
                .arg(&path)
                .spawn();
        }

        if let Some(path) = restore_backup {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "backup".to_string());
            match self.restore_backup(&path) {
                Ok(()) => {
                    self.push_toast(format!("Restored {name}"), ToastKind::Info);
                    close = true;
                }
                Err(error) => {
                    tools.message = Some(error);
                    tools.message_is_error = true;
                    tools.confirm_restore = None;
                }
            }
        }

        if !close {
            self.db_tools = Some(tools);
        }
    }

    pub(crate) fn toast_ui(&mut self, ctx: &egui::Context) {
        let p = self.palette;
        if self
            .toast
            .as_ref()
            .map_or(true, |t| Instant::now() > t.until)
        {
            self.toast = None;
            return;
        }
        let (text, kind) = {
            let toast = self.toast.as_ref().unwrap();
            (toast.text.clone(), toast.kind)
        };
        let color = match kind {
            ToastKind::Info => p.green,
            ToastKind::Error => p.red,
        };
        egui::Area::new(egui::Id::new("toast"))
            .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-16.0, -40.0))
            .show(ctx, |ui| {
                egui::Frame::none()
                    .fill(p.card)
                    .stroke(Stroke::new(1.0_f32, color))
                    .rounding(Rounding::same(7.0))
                    .inner_margin(Margin::symmetric(12.0, 8.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            widgets::dot(ui, color, 6.0);
                            ui.label(RichText::new(&text).size(11.6).color(p.text));
                        });
                    });
            });
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn line_offsets_match_the_editor() {
        let sql = "SELECT 1\nFROM t\nWHERE x = 1";
        assert_eq!(offset_of_line(sql, 1), 0);
        assert_eq!(offset_of_line(sql, 2), 9);
        assert_eq!(offset_of_line(sql, 3), 16);
        assert_eq!(offset_of_line(sql, 99), sql.len());
    }

    #[test]
    fn result_search_matches_any_column_case_insensitively() {
        let rows = vec![
            vec![Some("Avery".to_string()), Some("Singapore".to_string())],
            vec![Some("Mila".to_string()), Some("Czechia".to_string())],
            vec![None, Some("singapore".to_string())],
        ];
        let found = filter_query_rows(&rows, "SING");
        assert_eq!(found.len(), 2);
        assert!(filter_query_rows(&rows, "zzz").is_empty());
    }

    use super::*;
    use crate::theme;
    use std::cell::Cell;
    use std::rc::Rc;

    fn saved_connection() -> SavedConnection {
        SavedConnection {
            id: 1,
            name: "Demo".to_string(),
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
            group_id: Some(3),
        }
    }

    fn raw_input(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 600.0))),
            events,
            ..Default::default()
        }
    }

    #[test]
    fn cell_editor_preserves_null_and_empty_distinction() {
        assert_eq!(edited_cell_value(&None, ""), None);
        assert_eq!(edited_cell_value(&None, "abc"), Some("abc".to_string()));
        assert_eq!(
            edited_cell_value(&Some(String::new()), ""),
            Some(String::new())
        );
        assert_eq!(
            edited_cell_value(&Some("abc".to_string()), ""),
            Some(String::new())
        );
        assert_eq!(
            edited_cell_value(&Some("abc".to_string()), "xyz"),
            Some("xyz".to_string())
        );
    }

    /// Regression: the card's right-click background used to be registered on
    /// the whole frame after the Connect button, which could swallow primary
    /// clicks and leave Connect doing nothing.
    #[test]
    fn connect_button_in_a_grouped_card_receives_primary_click() {
        let ctx = egui::Context::default();
        let p = theme::apply(&ctx, false, 11.0, 11.0);
        let saved = saved_connection();
        let connected = Rc::new(Cell::new(false));
        let pos = egui::pos2(20.0 + 600.0 - 58.0, 20.0 + 29.0);

        let draw = |ctx: &egui::Context| {
            let p = p;
            let saved = &saved;
            let connected = connected.clone();
            egui::Area::new(egui::Id::new("connect-card-test"))
                .fixed_pos(egui::pos2(20.0, 20.0))
                .show(ctx, |ui| {
                    ui.set_width(600.0);
                    let (connect, _) = DataloomApp::connection_card(ui, &p, saved);
                    if connect {
                        connected.set(true);
                    }
                });
        };

        // Lay out the card, hover it, press and release the primary button.
        let _ = ctx.run(raw_input(Vec::new()), |ctx| draw(ctx));
        let _ = ctx.run(
            raw_input(vec![egui::Event::PointerMoved(pos)]),
            |ctx| draw(ctx),
        );
        let _ = ctx.run(
            raw_input(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::default(),
                },
            ]),
            |ctx| draw(ctx),
        );
        let _ = ctx.run(
            raw_input(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            }]),
            |ctx| draw(ctx),
        );

        assert!(connected.get(), "Connect button did not receive the click");
    }
}
