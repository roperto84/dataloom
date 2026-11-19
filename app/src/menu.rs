//! Application commands, the native macOS menu bar and the command palette
//! metadata.
//!
//! Every user-visible action is a [`CommandId`]. The native menu, the command
//! palette and the keyboard shortcuts all funnel through the same dispatcher,
//! so the three entry points cannot drift apart.

/// One action the app can run from the menu, the palette or a shortcut.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CommandId {
    NewConnection,
    NewQueryTab,
    CloseTab,
    Save,
    ExportSql,
    ImportSql,
    ImportData,
    Settings,
    Undo,
    Redo,
    Find,
    CommandPalette,
    RunQuery,
    Explain,
    FormatSql,
    BeginTransaction,
    CommitTransaction,
    RollbackTransaction,
    ToggleReadOnly,
    TemplateSelect,
    TemplateJoin,
    TemplateInsert,
    TemplateUpdate,
    TemplateDelete,
    TemplateCreateTable,
    ToggleTheme,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    ToggleRowDetail,
    DuplicateTab,
    About,
}

impl CommandId {
    /// Stable identifier used by the native menu.
    pub const fn id(self) -> &'static str {
        match self {
            CommandId::NewConnection => "cmd.new-connection",
            CommandId::NewQueryTab => "cmd.new-query",
            CommandId::CloseTab => "cmd.close-tab",
            CommandId::Save => "cmd.save",
            CommandId::ExportSql => "cmd.export-sql",
            CommandId::ImportSql => "cmd.import-sql",
            CommandId::ImportData => "cmd.import-data",
            CommandId::Settings => "cmd.settings",
            CommandId::Undo => "cmd.undo",
            CommandId::Redo => "cmd.redo",
            CommandId::Find => "cmd.find",
            CommandId::CommandPalette => "cmd.palette",
            CommandId::RunQuery => "cmd.run",
            CommandId::Explain => "cmd.explain",
            CommandId::FormatSql => "cmd.format",
            CommandId::BeginTransaction => "cmd.begin-transaction",
            CommandId::CommitTransaction => "cmd.commit-transaction",
            CommandId::RollbackTransaction => "cmd.rollback-transaction",
            CommandId::ToggleReadOnly => "cmd.toggle-read-only",
            CommandId::TemplateSelect => "cmd.template-select",
            CommandId::TemplateJoin => "cmd.template-join",
            CommandId::TemplateInsert => "cmd.template-insert",
            CommandId::TemplateUpdate => "cmd.template-update",
            CommandId::TemplateDelete => "cmd.template-delete",
            CommandId::TemplateCreateTable => "cmd.template-create-table",
            CommandId::ToggleTheme => "cmd.toggle-theme",
            CommandId::ZoomIn => "cmd.zoom-in",
            CommandId::ZoomOut => "cmd.zoom-out",
            CommandId::ZoomReset => "cmd.zoom-reset",
            CommandId::ToggleRowDetail => "cmd.row-detail",
            CommandId::DuplicateTab => "cmd.duplicate-tab",
            CommandId::About => "cmd.about",
        }
    }

    pub fn from_id(id: &str) -> Option<CommandId> {
        CommandId::ALL.iter().copied().find(|cmd| cmd.id() == id)
    }

    pub const ALL: &'static [CommandId] = &[
        CommandId::NewConnection,
        CommandId::NewQueryTab,
        CommandId::CloseTab,
        CommandId::DuplicateTab,
        CommandId::Save,
        CommandId::ExportSql,
        CommandId::ImportSql,
        CommandId::ImportData,
        CommandId::Settings,
        CommandId::Undo,
        CommandId::Redo,
        CommandId::Find,
        CommandId::CommandPalette,
        CommandId::RunQuery,
        CommandId::Explain,
        CommandId::FormatSql,
        CommandId::BeginTransaction,
        CommandId::CommitTransaction,
        CommandId::RollbackTransaction,
        CommandId::ToggleReadOnly,
        CommandId::TemplateSelect,
        CommandId::TemplateJoin,
        CommandId::TemplateInsert,
        CommandId::TemplateUpdate,
        CommandId::TemplateDelete,
        CommandId::TemplateCreateTable,
        CommandId::ToggleTheme,
        CommandId::ZoomIn,
        CommandId::ZoomOut,
        CommandId::ZoomReset,
        CommandId::ToggleRowDetail,
        CommandId::About,
    ];

    pub fn label(self) -> &'static str {
        match self {
            CommandId::NewConnection => "New connection",
            CommandId::NewQueryTab => "New query tab",
            CommandId::CloseTab => "Close tab",
            CommandId::DuplicateTab => "Duplicate tab",
            CommandId::Save => "Save",
            CommandId::ExportSql => "Export SQL…",
            CommandId::ImportSql => "Import SQL…",
            CommandId::ImportData => "Import data (CSV / Excel / JSON)…",
            CommandId::Settings => "Settings…",
            CommandId::Undo => "Undo",
            CommandId::Redo => "Redo",
            CommandId::Find => "Find",
            CommandId::CommandPalette => "Command palette",
            CommandId::RunQuery => "Run query",
            CommandId::Explain => "Explain query",
            CommandId::FormatSql => "Format SQL",
            CommandId::BeginTransaction => "Begin transaction",
            CommandId::CommitTransaction => "Commit transaction",
            CommandId::RollbackTransaction => "Roll back transaction",
            CommandId::ToggleReadOnly => "Toggle read-only mode",
            CommandId::TemplateSelect => "Template: SELECT",
            CommandId::TemplateJoin => "Template: JOIN",
            CommandId::TemplateInsert => "Template: INSERT",
            CommandId::TemplateUpdate => "Template: UPDATE",
            CommandId::TemplateDelete => "Template: DELETE",
            CommandId::TemplateCreateTable => "Template: CREATE TABLE",
            CommandId::ToggleTheme => "Toggle dark theme",
            CommandId::ZoomIn => "Increase font size",
            CommandId::ZoomOut => "Decrease font size",
            CommandId::ZoomReset => "Reset font size",
            CommandId::ToggleRowDetail => "Toggle row detail",
            CommandId::About => "About Dataloom",
        }
    }

    /// Human-readable shortcut, matching the accelerators installed in the
    /// native menu.
    pub fn shortcut(self) -> &'static str {
        match self {
            CommandId::NewConnection => "⌘N",
            CommandId::NewQueryTab => "⌘T",
            CommandId::CloseTab => "⌘W",
            CommandId::DuplicateTab => "Shift+Cmd+T",
            CommandId::Save => "⌘S",
            CommandId::ExportSql => "Shift+Cmd+E",
            CommandId::ImportSql => "Shift+Cmd+I",
            CommandId::ImportData => "",
            CommandId::Settings => "⌘,",
            CommandId::Undo => "⌘Z",
            CommandId::Redo => "Shift+Cmd+Z",
            CommandId::Find => "⌘F",
            CommandId::CommandPalette => "Shift+Cmd+P",
            CommandId::RunQuery => "Cmd+Enter",
            CommandId::Explain => "⌘E",
            CommandId::FormatSql => "Shift+Cmd+F",
            CommandId::ToggleReadOnly => "Shift+Cmd+L",
            CommandId::ToggleTheme => "Shift+Cmd+D",
            CommandId::ToggleRowDetail => "Space",
            _ => "",
        }
    }

    /// Commands that only make sense with a query tab open.
    pub fn needs_query_tab(self) -> bool {
        matches!(
            self,
            CommandId::RunQuery
                | CommandId::Explain
                | CommandId::FormatSql
                | CommandId::TemplateSelect
                | CommandId::TemplateJoin
                | CommandId::TemplateInsert
                | CommandId::TemplateUpdate
                | CommandId::TemplateDelete
                | CommandId::TemplateCreateTable
                | CommandId::BeginTransaction
                | CommandId::CommitTransaction
                | CommandId::RollbackTransaction
        )
    }

    /// Commands that only make sense once a connection is open.
    pub fn needs_connection(self) -> bool {
        matches!(
            self,
            CommandId::NewQueryTab
                | CommandId::CloseTab
                | CommandId::DuplicateTab
                | CommandId::ExportSql
                | CommandId::ImportSql
                | CommandId::ImportData
                | CommandId::Find
                | CommandId::ToggleRowDetail
                | CommandId::ToggleReadOnly
        ) || self.needs_query_tab()
    }
}

/// The native menu bar. Only built on macOS; the rest of the app keeps
/// working through shortcuts and the command palette.
#[cfg(target_os = "macos")]
pub struct NativeMenu {
    // Kept alive for the lifetime of the app: dropping it removes the menu.
    _menu: muda::Menu,
}

#[cfg(target_os = "macos")]
impl NativeMenu {
    /// Builds the menu bar and installs it as `NSApp.mainMenu`. Must run on the
    /// main thread, which `eframe`'s `update` is.
    pub fn install() -> Option<NativeMenu> {
        use muda::accelerator::{Accelerator, Code, Modifiers};
        use muda::{Menu, MenuItem, PredefinedMenuItem, Submenu};

        let accel = |mods: Modifiers, code: Code| Some(Accelerator::new(Some(mods), code));
        let cmd = Modifiers::SUPER;
        let shift_cmd = Modifiers::SUPER | Modifiers::SHIFT;

        let menu = Menu::new();
        let app_menu = Submenu::new("Dataloom", true);
        let settings = MenuItem::with_id(
            CommandId::Settings.id(),
            "Settings…",
            true,
            accel(cmd, Code::Comma),
        );
        let about = MenuItem::with_id(
            CommandId::About.id(),
            "About Dataloom",
            true,
            None::<Accelerator>,
        );
        let quit = PredefinedMenuItem::quit(Some("Quit Dataloom"));
        for item in [
            &settings as &dyn muda::IsMenuItem,
            &about,
            &PredefinedMenuItem::separator(),
            &quit,
        ] {
            let _ = app_menu.append(item);
        }
        let _ = menu.append(&app_menu);

        let file = Submenu::new("File", true);
        let new_connection = MenuItem::with_id(
            CommandId::NewConnection.id(),
            "New Connection…",
            true,
            accel(cmd, Code::KeyN),
        );
        let new_query = MenuItem::with_id(
            CommandId::NewQueryTab.id(),
            "New Query Tab",
            true,
            accel(cmd, Code::KeyT),
        );
        let close_tab = MenuItem::with_id(
            CommandId::CloseTab.id(),
            "Close Tab",
            true,
            accel(cmd, Code::KeyW),
        );
        let duplicate = MenuItem::with_id(
            CommandId::DuplicateTab.id(),
            "Duplicate Tab",
            true,
            accel(shift_cmd, Code::KeyT),
        );
        let save = MenuItem::with_id(
            CommandId::Save.id(),
            "Save",
            true,
            accel(cmd, Code::KeyS),
        );
        let export = MenuItem::with_id(
            CommandId::ExportSql.id(),
            "Export SQL…",
            true,
            accel(shift_cmd, Code::KeyE),
        );
        let import = MenuItem::with_id(
            CommandId::ImportSql.id(),
            "Import SQL…",
            true,
            accel(shift_cmd, Code::KeyI),
        );
        let import_data = MenuItem::with_id(
            CommandId::ImportData.id(),
            "Import Data (CSV / Excel / JSON)…",
            true,
            None::<Accelerator>,
        );
        for item in [
            &new_connection as &dyn muda::IsMenuItem,
            &new_query,
            &duplicate,
            &close_tab,
            &PredefinedMenuItem::separator(),
            &save,
            &PredefinedMenuItem::separator(),
            &export,
            &import,
            &import_data,
        ] {
            let _ = file.append(item);
        }
        let _ = menu.append(&file);

        let edit = Submenu::new("Edit", true);
        let undo = MenuItem::with_id(
            CommandId::Undo.id(),
            "Undo",
            true,
            accel(cmd, Code::KeyZ),
        );
        let redo = MenuItem::with_id(
            CommandId::Redo.id(),
            "Redo",
            true,
            accel(shift_cmd, Code::KeyZ),
        );
        let find = MenuItem::with_id(
            CommandId::Find.id(),
            "Find",
            true,
            accel(cmd, Code::KeyF),
        );
        let palette = MenuItem::with_id(
            CommandId::CommandPalette.id(),
            "Command Palette…",
            true,
            accel(shift_cmd, Code::KeyP),
        );
        for item in [
            &undo as &dyn muda::IsMenuItem,
            &redo,
            &PredefinedMenuItem::separator(),
            &find,
            &palette,
        ] {
            let _ = edit.append(item);
        }
        let _ = menu.append(&edit);

        let query = Submenu::new("Query", true);
        let run = MenuItem::with_id(
            CommandId::RunQuery.id(),
            "Run",
            true,
            accel(cmd, Code::Enter),
        );
        let explain = MenuItem::with_id(
            CommandId::Explain.id(),
            "Explain",
            true,
            accel(cmd, Code::KeyE),
        );
        let format = MenuItem::with_id(
            CommandId::FormatSql.id(),
            "Format SQL",
            true,
            accel(shift_cmd, Code::KeyF),
        );
        let templates = Submenu::new("Insert Template", true);
        for (cmd, label) in [
            (CommandId::TemplateSelect, "SELECT"),
            (CommandId::TemplateJoin, "JOIN"),
            (CommandId::TemplateInsert, "INSERT"),
            (CommandId::TemplateUpdate, "UPDATE"),
            (CommandId::TemplateDelete, "DELETE"),
            (CommandId::TemplateCreateTable, "CREATE TABLE"),
        ] {
            let item = MenuItem::with_id(cmd.id(), label, true, None::<Accelerator>);
            let _ = templates.append(&item);
        }
        let begin_tx = MenuItem::with_id(
            CommandId::BeginTransaction.id(),
            "Begin Transaction",
            true,
            None::<Accelerator>,
        );
        let commit_tx = MenuItem::with_id(
            CommandId::CommitTransaction.id(),
            "Commit Transaction",
            true,
            None::<Accelerator>,
        );
        let rollback_tx = MenuItem::with_id(
            CommandId::RollbackTransaction.id(),
            "Roll Back Transaction",
            true,
            None::<Accelerator>,
        );
        for item in [
            &run as &dyn muda::IsMenuItem,
            &explain,
            &format,
            &PredefinedMenuItem::separator(),
            &templates,
            &PredefinedMenuItem::separator(),
            &begin_tx,
            &commit_tx,
            &rollback_tx,
        ] {
            let _ = query.append(item);
        }
        let _ = menu.append(&query);

        let view = Submenu::new("View", true);
        let theme = MenuItem::with_id(
            CommandId::ToggleTheme.id(),
            "Toggle Dark Theme",
            true,
            accel(shift_cmd, Code::KeyD),
        );
        let row_detail = MenuItem::with_id(
            CommandId::ToggleRowDetail.id(),
            "Toggle Row Detail",
            true,
            None::<Accelerator>,
        );
        let read_only = MenuItem::with_id(
            CommandId::ToggleReadOnly.id(),
            "Read-only Mode",
            true,
            accel(shift_cmd, Code::KeyL),
        );
        let zoom_in = MenuItem::with_id(
            CommandId::ZoomIn.id(),
            "Increase Font Size",
            true,
            accel(cmd, Code::Equal),
        );
        let zoom_out = MenuItem::with_id(
            CommandId::ZoomOut.id(),
            "Decrease Font Size",
            true,
            accel(cmd, Code::Minus),
        );
        let zoom_reset = MenuItem::with_id(
            CommandId::ZoomReset.id(),
            "Reset Font Size",
            true,
            accel(cmd, Code::Digit0),
        );
        for item in [
            &theme as &dyn muda::IsMenuItem,
            &read_only,
            &row_detail,
            &PredefinedMenuItem::separator(),
            &zoom_in,
            &zoom_out,
            &zoom_reset,
        ] {
            let _ = view.append(item);
        }
        let _ = menu.append(&view);

        let help = Submenu::new("Help", true);
        let help_about = MenuItem::with_id(
            CommandId::About.id(),
            "About Dataloom",
            true,
            None::<Accelerator>,
        );
        let _ = help.append(&help_about);
        let _ = menu.append(&help);

        menu.init_for_nsapp();
        Some(NativeMenu { _menu: menu })
    }

    /// Non-blocking poll of the menu's event channel.
    pub fn poll() -> Option<CommandId> {
        let event = muda::MenuEvent::receiver().try_recv().ok()?;
        CommandId::from_id(event.id().0.as_str())
    }
}

#[cfg(not(target_os = "macos"))]
pub struct NativeMenu;

#[cfg(not(target_os = "macos"))]
impl NativeMenu {
    pub fn install() -> Option<NativeMenu> {
        None
    }

    pub fn poll() -> Option<CommandId> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_ids_are_unique_and_roundtrip() {
        let mut seen = std::collections::HashSet::new();
        for cmd in CommandId::ALL {
            assert!(seen.insert(cmd.id()), "duplicate id {}", cmd.id());
            assert_eq!(CommandId::from_id(cmd.id()), Some(*cmd));
        }
        assert_eq!(CommandId::from_id("nope"), None);
    }

    #[test]
    fn shortcuts_are_unique_except_explicit_context_ones() {
        let mut seen = std::collections::HashMap::new();
        for cmd in CommandId::ALL {
            let shortcut = cmd.shortcut();
            if shortcut.is_empty() || shortcut == "Space" {
                continue;
            }
            if let Some(previous) = seen.insert(shortcut, *cmd) {
                panic!(
                    "{} and {} share {shortcut}",
                    previous.label(),
                    cmd.label()
                );
            }
        }
    }

    #[test]
    fn query_commands_require_a_connection() {
        assert!(CommandId::RunQuery.needs_connection());
        assert!(!CommandId::RunQuery.needs_query_tab() || CommandId::RunQuery.needs_connection());
        assert!(!CommandId::About.needs_connection());
        assert!(!CommandId::Settings.needs_connection());
    }
}
