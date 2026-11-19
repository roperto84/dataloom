//! Persistent workspace preferences and the last session.
//!
//! Both files live in the app data folder next to `connections.json`:
//! `settings.json` for preferences and `session.json` for the tabs that were
//! open when the app closed. Loading is best-effort: a missing or malformed
//! file falls back to the defaults instead of blocking startup.

use crate::db::{self, SortSpec, TableFilter, TableId};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Row heights the settings panel offers. TablePlus stores an arbitrary
/// `NSRowHeight`, so the value is clamped rather than snapped.
pub const ROW_HEIGHT_OPTIONS: &[(f32, &str)] = &[
    (18.0, "Compact (18)"),
    (22.0, "Default (22)"),
    (26.0, "Comfortable (26)"),
    (30.0, "Tall (30)"),
];

pub const FONT_SIZE_OPTIONS: &[(f32, &str)] = &[
    (10.0, "10"),
    (11.0, "11"),
    (12.0, "12"),
    (13.0, "13"),
    (14.0, "14"),
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    #[serde(default)]
    pub dark: bool,
    /// Data grid font size in points.
    #[serde(default = "default_data_font")]
    pub data_font_size: f32,
    /// SQL editor font size in points.
    #[serde(default = "default_sql_font")]
    pub sql_font_size: f32,
    #[serde(default = "default_row_height")]
    pub row_height: f32,
    #[serde(default = "default_true")]
    pub alternate_rows: bool,
    #[serde(default = "default_true")]
    pub show_row_numbers: bool,
    #[serde(default = "default_tab_width")]
    pub tab_width: usize,
    /// Fraction of the query tab's height used by the editor.
    #[serde(default = "default_split")]
    pub editor_split: f32,
    #[serde(default = "default_true")]
    pub restore_session: bool,
    #[serde(default = "default_true")]
    pub confirm_close: bool,
    /// Ask before running statements that modify the database.
    #[serde(default)]
    pub safe_mode: bool,
}

fn default_data_font() -> f32 {
    crate::theme::DATA_FONT_SIZE
}

fn default_sql_font() -> f32 {
    crate::theme::SQL_FONT_SIZE
}

fn default_row_height() -> f32 {
    crate::theme::GRID_ROW_HEIGHT
}

fn default_tab_width() -> usize {
    crate::theme::TAB_WIDTH
}

fn default_split() -> f32 {
    0.40
}

fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            dark: crate::theme::DEFAULT_DARK,
            data_font_size: default_data_font(),
            sql_font_size: default_sql_font(),
            row_height: default_row_height(),
            alternate_rows: true,
            show_row_numbers: true,
            tab_width: default_tab_width(),
            editor_split: default_split(),
            restore_session: true,
            confirm_close: true,
            safe_mode: false,
        }
    }
}

impl Settings {
    /// Reads `settings.json`, clamping values that a hand-edited file could
    /// put out of range so the UI cannot be rendered unusable.
    pub fn load() -> Settings {
        Settings::load_from(&settings_path())
    }

    pub fn load_from(path: &Path) -> Settings {
        let mut settings = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Settings>(&text).ok())
            .unwrap_or_default();
        settings.normalize();
        settings
    }

    pub fn save(&self) {
        self.save_to(&settings_path());
    }

    pub fn save_to(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }

    pub fn normalize(&mut self) {
        self.data_font_size = self.data_font_size.clamp(9.0, 20.0);
        self.sql_font_size = self.sql_font_size.clamp(9.0, 20.0);
        self.row_height = self.row_height.clamp(16.0, 48.0);
        if !crate::theme::TAB_WIDTH_OPTIONS.contains(&self.tab_width) {
            self.tab_width = crate::theme::TAB_WIDTH;
        }
        self.editor_split = self.editor_split.clamp(0.15, 0.85);
    }

    pub fn tab_width_label(&self) -> String {
        format!("{} spaces", self.tab_width)
    }
}

fn settings_path() -> PathBuf {
    db::app_data_dir().join("settings.json")
}

fn session_path() -> PathBuf {
    db::app_data_dir().join("session.json")
}

/// One restorable tab. Table tabs store what `/sql` would need to reload them:
/// the table, paging window, filter and sort.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionTab {
    Query {
        #[serde(default)]
        title: String,
        #[serde(default)]
        sql: String,
        #[serde(default)]
        row_limit: usize,
        #[serde(default)]
        scroll: f32,
    },
    Table {
        schema: Option<String>,
        name: String,
        #[serde(default)]
        offset: usize,
        #[serde(default)]
        limit: usize,
        #[serde(default)]
        filter: TableFilter,
        #[serde(default)]
        sort: Vec<SortSpec>,
        #[serde(default)]
        detail_open: bool,
        #[serde(default)]
        scroll: f32,
    },
}

impl SessionTab {
    pub fn query(title: String, sql: String, row_limit: usize, scroll: f32) -> Self {
        SessionTab::Query {
            title,
            sql,
            row_limit,
            scroll,
        }
    }

    pub fn table(
        table: &TableId,
        offset: usize,
        limit: usize,
        filter: &TableFilter,
        sort: &[SortSpec],
        detail_open: bool,
        scroll: f32,
    ) -> Self {
        SessionTab::Table {
            schema: table.schema.clone(),
            name: table.name.clone(),
            offset,
            limit,
            filter: filter.clone(),
            sort: sort.to_vec(),
            detail_open,
            scroll,
        }
    }

}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Session {
    #[serde(default)]
    pub connection_id: Option<u64>,
    #[serde(default)]
    pub connection_name: String,
    #[serde(default)]
    pub active: usize,
    #[serde(default)]
    pub tabs: Vec<SessionTab>,
}

impl Session {
    pub fn load() -> Session {
        Session::load_from(&session_path())
    }

    pub fn load_from(path: &Path) -> Session {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Session>(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        self.save_to(&session_path());
    }

    pub fn save_to(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, text);
        }
    }

    pub fn clear() {
        let _ = std::fs::remove_file(session_path());
    }

    pub fn is_empty(&self) -> bool {
        self.tabs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dataloom-settings-{}-{label}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn settings_roundtrip_and_clamping() {
        let dir = temp_dir("settings");
        let path = dir.join("settings.json");
        let settings = Settings {
            dark: true,
            data_font_size: 13.0,
            row_height: 26.0,
            tab_width: 8,
            editor_split: 0.55,
            safe_mode: true,
            ..Settings::default()
        };
        settings.save_to(&path);

        let loaded = Settings::load_from(&path);
        assert_eq!(loaded, settings);

        // A hand-edited file with out-of-range values is clamped on load.
        std::fs::write(
            &path,
            r#"{"data_font_size":200,"sql_font_size":1,"row_height":2,"tab_width":5,"editor_split":9}"#,
        )
        .unwrap();
        let clamped = Settings::load_from(&path);
        assert_eq!(clamped.data_font_size, 20.0);
        assert_eq!(clamped.sql_font_size, 9.0);
        assert_eq!(clamped.row_height, 16.0);
        assert_eq!(clamped.tab_width, crate::theme::TAB_WIDTH);
        assert_eq!(clamped.editor_split, 0.85);

        // Missing file: defaults, not an empty struct.
        let missing = Settings::load_from(&dir.join("absent.json"));
        assert_eq!(missing, Settings::default());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn session_roundtrip_keeps_query_and_table_state() {
        let dir = temp_dir("session");
        let path = dir.join("session.json");
        let table = TableId {
            schema: Some("analytics".to_string()),
            name: "events".to_string(),
        };
        let filter = TableFilter {
            quick: "click".to_string(),
            columns: vec![db::ColumnFilter {
                column: "kind".to_string(),
                op: db::FilterOp::Equals,
                value: "click".to_string(),
            }],
            any: false,
        };
        let session = Session {
            connection_id: Some(7),
            connection_name: "Production".to_string(),
            active: 1,
            tabs: vec![
                SessionTab::query("Query 1".to_string(), "SELECT 1".to_string(), 500, 42.0),
                SessionTab::table(
                    &table,
                    200,
                    300,
                    &filter,
                    &[SortSpec {
                        column: "kind".to_string(),
                        descending: true,
                    }],
                    true,
                    120.0,
                ),
            ],
        };
        session.save_to(&path);

        let loaded = Session::load_from(&path);
        assert_eq!(loaded, session);

        std::fs::write(&path, "not json").unwrap();
        assert!(Session::load_from(&path).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }
}
