use mysql::prelude::Queryable;
use mysql::{OptsBuilder, Row as MyRow, SslOpts, Value as MyValue};
use postgres::NoTls;
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::time::Instant;

// ---------------------------------------------------------------------------
// Connections
// ---------------------------------------------------------------------------

/// A saved connection. `group_id` is an optional folder id in
/// `Store::groups`; older stores deserialize with no folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedConnection {
    #[serde(default)]
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub path: String,
    #[serde(default = "default_driver")]
    pub driver: String,
    #[serde(default)]
    pub host: String,
    #[serde(default)]
    pub port: u16,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub database: String,
    /// Open the connection without allowing writes.
    #[serde(default)]
    pub read_only: bool,
    #[serde(default = "default_ssl_mode")]
    pub ssl_mode: String,
    #[serde(default)]
    pub ssl_root_cert: String,
    #[serde(default)]
    pub ssh_host: String,
    #[serde(default)]
    pub ssh_port: u16,
    #[serde(default)]
    pub ssh_user: String,
    #[serde(default)]
    pub ssh_key_path: String,
    #[serde(default)]
    pub group_id: Option<u64>,
}

fn default_ssl_mode() -> String {
    "disable".to_string()
}

/// A connection folder ("group"/"project"). Flat, one level, like the
/// top-level groups in TablePlus's connection picker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionGroup {
    #[serde(default)]
    pub id: u64,
    pub name: String,
}

fn default_driver() -> String {
    "SQLite".to_string()
}

/// The database engines this workspace can talk to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Driver {
    Sqlite,
    MySql,
    Postgres,
}

impl Driver {
    pub fn label(self) -> &'static str {
        match self {
            Driver::Sqlite => "SQLite",
            Driver::MySql => "MySQL",
            Driver::Postgres => "PostgreSQL",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Driver::Sqlite => 0,
            Driver::MySql => 3306,
            Driver::Postgres => 5432,
        }
    }

    pub fn from_name(name: &str) -> Driver {
        match name.trim().to_ascii_lowercase().as_str() {
            "mysql" | "mariadb" => Driver::MySql,
            "postgres" | "postgresql" | "psql" | "pg" => Driver::Postgres,
            _ => Driver::Sqlite,
        }
    }

    pub fn all() -> [Driver; 3] {
        [Driver::Sqlite, Driver::MySql, Driver::Postgres]
    }
}

impl SavedConnection {
    pub fn driver_kind(&self) -> Driver {
        Driver::from_name(&self.driver)
    }

    /// The one-line target shown in cards and the status bar.
    pub fn target(&self) -> String {
        match self.driver_kind() {
            Driver::Sqlite => self.path.clone(),
            Driver::MySql | Driver::Postgres => {
                let port = self.port;
                let host = if self.host.is_empty() {
                    "127.0.0.1"
                } else {
                    &self.host
                };
                if self.database.is_empty() {
                    format!("{}@{}:{}", self.user, host, port)
                } else {
                    format!("{}@{}:{}/{}", self.user, host, port, self.database)
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Store {
    #[serde(default)]
    pub connections: Vec<SavedConnection>,
    #[serde(default)]
    pub groups: Vec<ConnectionGroup>,
    #[serde(default)]
    pub last_id: Option<u64>,
}

pub fn app_data_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join("Library")
        .join("Application Support")
        .join("Dataloom")
}

fn store_path() -> PathBuf {
    app_data_dir().join("connections.json")
}

const KEYCHAIN_SERVICE: &str = "com.dataloom.app";

fn password_account(connection_id: u64) -> String {
    format!("connection-{connection_id}")
}

/// Stores a connection password in the macOS Keychain. On other platforms
/// there is no Keychain implementation here, so the caller keeps the password
/// in its in-memory record.
#[cfg(target_os = "macos")]
pub fn store_password(connection_id: u64, password: &str) -> Result<(), String> {
    security_framework::passwords::set_generic_password(
        KEYCHAIN_SERVICE,
        &password_account(connection_id),
        password.as_bytes(),
    )
    .map_err(|error| format!("Could not store the password in the Keychain: {error}"))
}

#[cfg(not(target_os = "macos"))]
pub fn store_password(_connection_id: u64, _password: &str) -> Result<(), String> {
    Err("The Keychain is only available on macOS.".to_string())
}

#[cfg(target_os = "macos")]
pub fn load_password(connection_id: u64) -> Option<String> {
    let options = security_framework::passwords::PasswordOptions::new_generic_password(
        KEYCHAIN_SERVICE,
        &password_account(connection_id),
    );
    security_framework::passwords::generic_password(options)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
}

#[cfg(not(target_os = "macos"))]
pub fn load_password(_connection_id: u64) -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
pub fn delete_password(connection_id: u64) {
    let _ = security_framework::passwords::delete_generic_password(
        KEYCHAIN_SERVICE,
        &password_account(connection_id),
    );
}

#[cfg(not(target_os = "macos"))]
pub fn delete_password(_connection_id: u64) {}

/// True once a saved-connection store has been written, even an empty one.
pub fn store_exists() -> bool {
    store_path().exists()
}

/// The sample connection is seeded on the very first launch only. On later
/// launches an empty store means the user deleted their connections on
/// purpose, so the sample must not be resurrected.
pub fn should_seed_sample(store: &Store, store_file_exists: bool) -> bool {
    store.connections.is_empty() && !store_file_exists
}

// ---------------------------------------------------------------------------
// Backups and dump paths
// ---------------------------------------------------------------------------

/// One backup file on disk.
#[derive(Clone, Debug)]
pub struct BackupFile {
    pub path: PathBuf,
    pub name: String,
    pub bytes: u64,
    /// Unix seconds of the last modification, UTC.
    pub modified: u64,
}

/// Folder that holds the backups of one saved connection. It lives in the app
/// data folder so restoring a database never overwrites its own backups.
pub fn backups_dir(saved: &SavedConnection) -> PathBuf {
    app_data_dir()
        .join("backups")
        .join(sanitize_file_stem(&saved.name))
}

/// Creates a backup inside `dir`: a byte-for-byte copy for SQLite and a SQL
/// dump for the server engines. Returns the new file.
pub fn create_backup_in(dir: &Path, saved: &SavedConnection, db: &Db) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let stem = sanitize_file_stem(&saved.name);
    let stamp = timestamp_slug();
    match db.driver {
        Driver::Sqlite => {
            let source = Path::new(&saved.path);
            if !source.is_file() {
                return Err(format!(
                    "The database file {} is missing.",
                    source.display()
                ));
            }
            let dest = dir.join(format!("{stem}-{stamp}.db"));
            fs::copy(source, &dest)
                .map_err(|e| format!("Could not copy the database file: {e}"))?;
            Ok(dest)
        }
        _ => {
            let dest = dir.join(format!("{stem}-{stamp}.sql"));
            let dump = db.dump_database_sql(&saved.name)?;
            fs::write(&dest, dump).map_err(|e| format!("Could not write the dump: {e}"))?;
            Ok(dest)
        }
    }
}

pub fn list_backups(saved: &SavedConnection) -> Vec<BackupFile> {
    list_backups_in(&backups_dir(saved))
}

pub fn list_backups_in(dir: &Path) -> Vec<BackupFile> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().to_string();
        if !(name.ends_with(".db") || name.ends_with(".sql")) {
            continue;
        }
        out.push(BackupFile {
            path,
            name,
            bytes: meta.len(),
            modified: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0),
        });
    }
    out.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| b.name.cmp(&a.name))
    });
    out
}

pub fn delete_backup(path: &Path) -> Result<(), String> {
    fs::remove_file(path).map_err(|e| format!("Could not delete {}: {e}", path.display()))
}

/// Suggested export path: the user's Desktop when it exists, else the home
/// folder.
pub fn default_dump_path(stem: &str) -> PathBuf {
    export_path(stem, "sql")
}

/// Default path for a table export with the given extension (`sql`, `csv`,
/// `xlsx`): `<Desktop or home>/<table>-<timestamp>.<ext>`.
pub fn export_path(stem: &str, extension: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let desktop = PathBuf::from(&home).join("Desktop");
    let dir = if desktop.is_dir() {
        desktop
    } else {
        PathBuf::from(home)
    };
    dir.join(format!(
        "{}-{}.{}",
        sanitize_file_stem(stem),
        timestamp_slug(),
        extension
    ))
}

/// Path component that keeps letters, digits, dashes and underscores.
pub fn sanitize_file_stem(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_dash = false;
    for c in name.chars() {
        if c.is_alphanumeric() || c == '_' {
            out.push(c);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "dataloom".to_string()
    } else {
        trimmed
    }
}

/// A `CREATE` statement with exactly one trailing semicolon.
fn with_terminator(ddl: &str) -> String {
    let trimmed = ddl.trim().trim_end_matches(';').trim_end();
    format!("{trimmed};")
}

/// `2026-11-10-1725-04`, UTC. Used for backup and export file names.
pub fn timestamp_slug() -> String {
    let (year, month, day, hour, minute, second) = utc_parts(now_secs());
    format!("{year:04}-{month:02}-{day:02}-{hour:02}{minute:02}-{second:02}")
}

/// Human readable UTC timestamp for dump headers.
pub fn utc_stamp() -> String {
    let (year, month, day, hour, minute, second) = utc_parts(now_secs());
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02} UTC")
}

/// Today's civil date and current UTC time.
pub fn now_parts() -> (i32, u32, u32, u32, u32, u32) {
    utc_parts(now_secs())
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Howard Hinnant's civil-from-days algorithm, so file names need no date
/// library.
pub fn utc_parts(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        (if month <= 2 { year + 1 } else { year }) as i32,
        month as u32,
        day as u32,
        hour as u32,
        minute as u32,
        second as u32,
    )
}

impl Store {
    pub fn load() -> Store {
        let path = store_path();
        if let Ok(text) = fs::read_to_string(&path) {
            if let Ok(mut store) = serde_json::from_str::<Store>(&text) {
                // Migrate legacy plaintext passwords to the Keychain.
                if store
                    .connections
                    .iter()
                    .any(|connection| connection.id != 0 && !connection.password.is_empty())
                {
                    store.save();
                }
                return store;
            }
        }
        Store::default()
    }

    pub fn save(&mut self) {
        let dir = app_data_dir();
        let _ = fs::create_dir_all(&dir);
        // Secrets never go to the JSON file: move them to the Keychain and
        // clear the in-memory field. If the Keychain is unavailable the
        // password is kept so the connection still works.
        for connection in &mut self.connections {
            if connection.id != 0 && !connection.password.is_empty() {
                match store_password(connection.id, &connection.password) {
                    Ok(()) => connection.password.clear(),
                    Err(error) => eprintln!("{error}"),
                }
            }
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = fs::write(store_path(), text);
        }
    }

    pub fn next_id(&self) -> u64 {
        self.connections
            .iter()
            .map(|c| c.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    pub fn add(&mut self, name: &str, path: &str) -> SavedConnection {
        let saved = SavedConnection {
            id: self.next_id(),
            name: name.to_string(),
            path: path.to_string(),
            driver: default_driver(),
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
        };
        self.last_id = Some(saved.id);
        self.connections.push(saved.clone());
        self.save();
        saved
    }

    /// Adds several connections and writes the store once. Used by connection
    /// import so a large file is not a full rewrite per entry.
    pub fn add_connections(
        &mut self,
        mut connections: Vec<SavedConnection>,
    ) -> Vec<SavedConnection> {
        let mut next = self.next_id();
        for connection in &mut connections {
            connection.id = next;
            next = next.saturating_add(1);
        }
        if let Some(last) = connections.last() {
            self.last_id = Some(last.id);
        }
        self.connections.extend(connections.iter().cloned());
        self.save();
        connections
    }

    /// Adds a fully described connection (any driver) to the store.
    pub fn add_connection(&mut self, mut saved: SavedConnection) -> SavedConnection {
        saved.id = self.next_id();
        self.last_id = Some(saved.id);
        self.connections.push(saved.clone());
        self.save();
        saved
    }

    pub fn update_connection(&mut self, saved: SavedConnection) {
        if let Some(existing) = self.connections.iter_mut().find(|c| c.id == saved.id) {
            *existing = saved;
            self.save();
        }
    }

    pub fn delete_connection(&mut self, id: u64) {
        self.connections.retain(|c| c.id != id);
        if self.last_id == Some(id) {
            self.last_id = self.connections.first().map(|c| c.id);
        }
        delete_password(id);
        self.save();
    }

    pub fn next_group_id(&self) -> u64 {
        self.groups
            .iter()
            .map(|group| group.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    pub fn add_group(&mut self, name: &str) -> ConnectionGroup {
        let group = ConnectionGroup {
            id: self.next_group_id(),
            name: name.trim().to_string(),
        };
        self.groups.push(group.clone());
        self.save();
        group
    }

    pub fn rename_group(&mut self, id: u64, name: &str) {
        if let Some(group) = self.groups.iter_mut().find(|group| group.id == id) {
            group.name = name.trim().to_string();
            self.save();
        }
    }

    /// Removes a group without touching its connections: they fall back to the
    /// ungrouped list.
    pub fn delete_group(&mut self, id: u64) {
        self.groups.retain(|group| group.id != id);
        for connection in &mut self.connections {
            if connection.group_id == Some(id) {
                connection.group_id = None;
            }
        }
        self.save();
    }

    /// Moves a connection into a group (or out of every group with `None`).
    pub fn set_connection_group(&mut self, connection_id: u64, group_id: Option<u64>) {
        if let Some(connection) = self
            .connections
            .iter_mut()
            .find(|connection| connection.id == connection_id)
        {
            connection.group_id = group_id;
            self.save();
        }
    }
}

pub fn sample_db_path() -> PathBuf {
    app_data_dir().join("sample-shop.db")
}

fn history_path() -> PathBuf {
    app_data_dir().join("query_history.json")
}

pub fn load_query_history() -> Vec<String> {
    fs::read_to_string(history_path())
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
        .unwrap_or_default()
}

pub fn save_query_history(items: &[String]) {
    let dir = app_data_dir();
    let _ = fs::create_dir_all(&dir);
    if let Ok(text) = serde_json::to_string_pretty(items) {
        let _ = fs::write(history_path(), text);
    }
}

pub fn open_sqlite(saved: &SavedConnection) -> Result<Connection, String> {
    let path = Path::new(&saved.path);
    if !path.exists() {
        return Err(format!("The file does not exist: {}", saved.path));
    }
    let conn = Connection::open(path).map_err(|e| format!("Could not open the database: {e}"))?;
    let _ = conn.busy_timeout(std::time::Duration::from_millis(3000));
    let _ = conn.execute_batch("PRAGMA foreign_keys = ON;");
    Ok(conn)
}

// ---------------------------------------------------------------------------
// Unified connection: SQLite, MySQL and PostgreSQL behind one interface
// ---------------------------------------------------------------------------

pub enum Backend {
    Sqlite(Connection),
    MySql(RefCell<mysql::Conn>),
    Postgres(RefCell<postgres::Client>),
}

pub struct Db {
    pub driver: Driver,
    backend: Backend,
    /// When set, statements that modify the database are refused. Shared with
    /// the toolbar toggle and the query worker through the connection record.
    read_only: AtomicBool,
}

/// Connection parameters shared by the connection itself and the separate
/// connection used to run `KILL QUERY` for cancellation.
fn mysql_options(saved: &SavedConnection) -> OptsBuilder {
    let host = if saved.host.trim().is_empty() {
        "127.0.0.1"
    } else {
        saved.host.trim()
    };
    let port = if saved.port == 0 { 3306 } else { saved.port };
    let mut builder = OptsBuilder::new()
        .ip_or_hostname(Some(host.to_string()))
        .tcp_port(port);
    if !saved.user.trim().is_empty() {
        builder = builder.user(Some(saved.user.trim().to_string()));
    }
    if !saved.password.is_empty() {
        builder = builder.pass(Some(saved.password.clone()));
    }
    if !saved.database.trim().is_empty() {
        builder = builder.db_name(Some(saved.database.trim().to_string()));
    }
    builder
}

/// TLS mode stored on a connection. The strings are stable in the JSON file.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SslMode {
    Disable,
    Require,
    VerifyCa,
    VerifyFull,
}

fn ssl_mode(saved: &SavedConnection) -> SslMode {
    match saved.ssl_mode.trim().to_ascii_lowercase().as_str() {
        "require" => SslMode::Require,
        "verify-ca" => SslMode::VerifyCa,
        "verify-full" => SslMode::VerifyFull,
        _ => SslMode::Disable,
    }
}

fn connection_path(text: &str) -> PathBuf {
    let text = text.trim();
    if let Some(rest) = text.strip_prefix("~/") {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(rest)
    } else {
        PathBuf::from(text)
    }
}

fn load_root_certificates(path: &str) -> Result<Vec<native_tls::Certificate>, String> {
    let path_buf = connection_path(path);
    let path = path_buf.to_string_lossy();
    let bytes = fs::read(&*path_buf)
        .map_err(|error| format!("Could not read the SSL root certificate {path}: {error}"))?;
    if let Ok(certificates) = native_tls::Certificate::stack_from_pem(&bytes) {
        if !certificates.is_empty() {
            return Ok(certificates);
        }
    }
    native_tls::Certificate::from_der(&bytes)
        .map(|certificate| vec![certificate])
        .map_err(|error| format!("Could not parse the SSL root certificate {path}: {error}"))
}

fn mysql_ssl_opts(saved: &SavedConnection) -> Result<Option<SslOpts>, String> {
    let mode = ssl_mode(saved);
    if mode == SslMode::Disable {
        return Ok(None);
    }
    let mut opts = SslOpts::default();
    match mode {
        SslMode::Require => {
            // Encrypted transport without certificate verification.
            opts = opts
                .with_danger_accept_invalid_certs(true)
                .with_danger_skip_domain_validation(true);
        }
        SslMode::VerifyCa => {
            opts = opts.with_danger_skip_domain_validation(true);
        }
        SslMode::VerifyFull => {}
        SslMode::Disable => unreachable!(),
    }
    if !saved.ssl_root_cert.trim().is_empty() {
        let path = connection_path(&saved.ssl_root_cert);
        if !path.is_file() {
            return Err(format!(
                "The SSL root certificate does not exist: {}",
                path.display()
            ));
        }
        opts = opts.with_root_cert_path(Some(path));
    }
    Ok(Some(opts))
}

fn postgres_tls_connector(
    saved: &SavedConnection,
    mode: SslMode,
) -> Result<native_tls::TlsConnector, String> {
    let mut builder = native_tls::TlsConnector::builder();
    match mode {
        SslMode::Require => {
            builder.danger_accept_invalid_certs(true);
            builder.danger_accept_invalid_hostnames(true);
        }
        SslMode::VerifyCa => {
            // Verify the chain against the CA, but not the host name: an SSH
            // tunnel connects to 127.0.0.1 while the certificate names the
            // database host.
            builder.danger_accept_invalid_hostnames(true);
        }
        SslMode::VerifyFull => {}
        SslMode::Disable => unreachable!(),
    }
    if !saved.ssl_root_cert.trim().is_empty() {
        for certificate in load_root_certificates(&saved.ssl_root_cert)? {
            builder.add_root_certificate(certificate);
        }
    }
    builder
        .build()
        .map_err(|error| format!("Could not build the TLS connector: {error}"))
}

/// A handle that can stop the query currently running on a [`Db`]. The handle
/// is sent to the UI thread before the query starts; cancelling from another
/// thread is what makes the Stop button possible while the query blocks.
pub enum QueryCancel {
    Sqlite(rusqlite::InterruptHandle),
    Postgres(postgres::CancelToken),
    MySql {
        saved: SavedConnection,
        connection_id: u32,
    },
}

impl QueryCancel {
    /// Cancels the query. SQLite interruption is immediate; PostgreSQL and
    /// MySQL use a short extra connection, so the UI calls this off-thread.
    pub fn cancel(self) -> Result<(), String> {
        match self {
            QueryCancel::Sqlite(handle) => {
                handle.interrupt();
                Ok(())
            }
            QueryCancel::Postgres(token) => token
                .cancel_query(NoTls)
                .map_err(|error| format!("Could not cancel the PostgreSQL query: {error}")),
            QueryCancel::MySql {
                saved,
                connection_id,
            } => {
                let mut conn = mysql::Conn::new(mysql_options(&saved))
                    .map_err(|error| format!("Could not connect to cancel the MySQL query: {error}"))?;
                conn.query_drop(format!("KILL QUERY {connection_id}"))
                    .map_err(|error| format!("Could not cancel the MySQL query: {error}"))
            }
        }
    }
}

impl Db {
    pub fn open(saved: &SavedConnection) -> Result<Db, String> {
        if !saved.ssh_host.trim().is_empty() && ssl_mode(saved) == SslMode::VerifyFull {
            return Err(
                "TLS verify-full cannot verify the certificate host name through an SSH tunnel; use verify-ca instead."
                    .to_string(),
            );
        }
        match saved.driver_kind() {
            Driver::Sqlite => Ok(Db {
                driver: Driver::Sqlite,
                backend: Backend::Sqlite(open_sqlite(saved)?),
                read_only: AtomicBool::new(saved.read_only),
            }),
            Driver::MySql => {
                let mut builder = mysql_options(saved);
                if let Some(opts) = mysql_ssl_opts(saved)? {
                    builder = builder.ssl_opts(Some(opts));
                }
                let conn = mysql::Conn::new(builder)
                    .map_err(|e| format!("Could not connect to MySQL: {e}"))?;
                Ok(Db {
                    driver: Driver::MySql,
                    backend: Backend::MySql(RefCell::new(conn)),
                    read_only: AtomicBool::new(saved.read_only),
                })
            }
            Driver::Postgres => {
                let host = if saved.host.trim().is_empty() {
                    "127.0.0.1"
                } else {
                    saved.host.trim()
                };
                let port = if saved.port == 0 { 5432 } else { saved.port };
                let mut config = postgres::Config::new();
                config.host(host).port(port);
                if !saved.user.trim().is_empty() {
                    config.user(saved.user.trim());
                }
                if !saved.password.is_empty() {
                    config.password(saved.password.clone());
                }
                if !saved.database.trim().is_empty() {
                    config.dbname(saved.database.trim());
                }
                let client = if ssl_mode(saved) == SslMode::Disable {
                    config
                        .connect(NoTls)
                        .map_err(|e| format!("Could not connect to PostgreSQL: {e}"))?
                } else {
                    let connector = postgres_tls_connector(saved, ssl_mode(saved))?;
                    config
                        .connect(postgres_native_tls::MakeTlsConnector::new(connector))
                        .map_err(|e| format!("Could not connect to PostgreSQL: {e}"))?
                };
                Ok(Db {
                    driver: Driver::Postgres,
                    backend: Backend::Postgres(RefCell::new(client)),
                    read_only: AtomicBool::new(saved.read_only),
                })
            }
        }
    }

    pub fn quote(&self, name: &str) -> String {
        let (schema, name) = split_table_id(name);
        match self.driver {
            Driver::MySql => match schema {
                Some(schema) if !schema.is_empty() => format!(
                    "`{}`.`{}`",
                    schema.replace('`', "``"),
                    name.replace('`', "``")
                ),
                _ => format!("`{}`", name.replace('`', "``")),
            },
            _ => match schema {
                Some(schema) if !schema.is_empty() => {
                    format!("{}.{}", quote_ident(schema), quote_ident(name))
                }
                _ => quote_ident(name),
            },
        }
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only.load(AtomicOrdering::Relaxed)
    }

    pub fn set_read_only(&self, value: bool) {
        self.read_only.store(value, AtomicOrdering::Relaxed);
    }

    fn ensure_writable(&self, action: &str) -> Result<(), String> {
        if self.is_read_only() {
            Err(format!(
                "Read-only mode is on: cannot {action}. Turn it off in the toolbar first."
            ))
        } else {
            Ok(())
        }
    }

    /// A cancellation handle for a query about to run on this connection.
    /// `saved` is only needed to open the separate MySQL connection that runs
    /// `KILL QUERY`.
    pub fn query_cancel_handle(&self, saved: &SavedConnection) -> QueryCancel {
        match &self.backend {
            Backend::Sqlite(conn) => QueryCancel::Sqlite(conn.get_interrupt_handle()),
            Backend::MySql(_) => {
                let conn = self.mysql();
                QueryCancel::MySql {
                    saved: saved.clone(),
                    connection_id: conn.connection_id(),
                }
            }
            Backend::Postgres(_) => {
                let client = self.pg();
                QueryCancel::Postgres(client.cancel_token())
            }
        }
    }

    fn mysql(&self) -> std::cell::RefMut<'_, mysql::Conn> {
        match &self.backend {
            Backend::MySql(conn) => conn.borrow_mut(),
            _ => unreachable!("not a MySQL connection"),
        }
    }

    fn pg(&self) -> std::cell::RefMut<'_, postgres::Client> {
        match &self.backend {
            Backend::Postgres(client) => client.borrow_mut(),
            _ => unreachable!("not a PostgreSQL connection"),
        }
    }

    pub fn list_tables(&self) -> Result<Vec<TableInfo>, String> {
        match &self.backend {
            Backend::Sqlite(conn) => list_tables(conn),
            Backend::MySql(_) => {
                let mut conn = self.mysql();
                let rows: Vec<MyRow> = conn
                    .query(
                        "SELECT table_name, table_type, table_rows
                         FROM information_schema.tables
                         WHERE table_schema = DATABASE()
                         ORDER BY table_type, table_name",
                    )
                    .map_err(|e| e.to_string())?;
                let mut out = Vec::new();
                for row in rows {
                    let name = mysql_string(&row, 0).unwrap_or_default();
                    let kind = mysql_string(&row, 1).unwrap_or_default();
                    let is_view = kind.to_ascii_uppercase().contains("VIEW");
                    let rows = if is_view { None } else { mysql_i64(&row, 2) };
                    out.push(TableInfo {
                        name,
                        schema: None,
                        is_view,
                        rows,
                    });
                }
                Ok(out)
            }
            Backend::Postgres(_) => {
                let rows = self.pg_rows(
                    "SELECT c.relname,
                            CASE WHEN c.relkind IN ('v','m') THEN 'VIEW' ELSE 'BASE TABLE' END,
                            CASE WHEN c.relkind IN ('v','m') THEN NULL ELSE c.reltuples::bigint END,
                            n.nspname,
                            (n.nspname = current_schema()) AS is_current
                     FROM pg_class c
                     JOIN pg_namespace n ON n.oid = c.relnamespace
                     WHERE c.relkind IN ('r','p','v','m')
                       AND n.nspname NOT IN ('pg_catalog','information_schema')
                     ORDER BY is_current DESC, n.nspname, 2, 1",
                )?;
                let mut out = Vec::new();
                for row in rows {
                    let kind = row.get(1).cloned().flatten().unwrap_or_default();
                    let is_view = kind.to_ascii_uppercase().contains("VIEW");
                    let rows = if is_view {
                        None
                    } else {
                        row.get(2)
                            .and_then(|v| v.as_deref())
                            .and_then(|v| v.parse::<i64>().ok())
                            .filter(|v| *v >= 0)
                    };
                    let schema_name = row.get(3).cloned().flatten();
                    let is_current = row
                        .get(4)
                        .and_then(|v| v.as_deref())
                        .map(|v| v == "t" || v.eq_ignore_ascii_case("true"))
                        .unwrap_or(false);
                    // Tables in the current schema stay unqualified so the
                    // sidebar and tabs show plain names; tables elsewhere are
                    // prefixed to keep duplicate names distinguishable.
                    let schema = if is_current { None } else { schema_name };
                    out.push(TableInfo {
                        name: row.first().cloned().flatten().unwrap_or_default(),
                        schema,
                        is_view,
                        rows,
                    });
                }
                Ok(out)
            }
        }
    }

    pub fn table_structure(&self, table: &str) -> Result<Vec<ColumnMeta>, String> {
        match &self.backend {
            Backend::Sqlite(conn) => table_structure(conn, table),
            Backend::MySql(_) => {
                let mut conn = self.mysql();
                let rows: Vec<MyRow> = conn
                    .exec(
                        "SELECT column_name, column_type, is_nullable, column_default, column_key
                         FROM information_schema.columns
                         WHERE table_schema = DATABASE() AND table_name = ?
                         ORDER BY ordinal_position",
                        (table,),
                    )
                    .map_err(|e| e.to_string())?;
                let mut out = Vec::new();
                for row in rows {
                    let nullable = mysql_string(&row, 2).unwrap_or_default();
                    let key = mysql_string(&row, 4).unwrap_or_default();
                    let decl_type = mysql_string(&row, 1).unwrap_or_default();
                    let options = parse_enum_type(&decl_type);
                    out.push(ColumnMeta {
                        name: mysql_string(&row, 0).unwrap_or_default(),
                        decl_type,
                        not_null: nullable.eq_ignore_ascii_case("NO"),
                        default_value: mysql_string(&row, 3),
                        primary_key: key.eq_ignore_ascii_case("PRI"),
                        options,
                    });
                }
                Ok(out)
            }
            Backend::Postgres(_) => {
                let (table_name, schema_sql) = pg_relation_predicate(table);
                let sql = format!(
                    "SELECT a.attname,
                            format_type(a.atttypid, a.atttypmod),
                            a.attnotnull,
                            pg_get_expr(d.adbin, d.adrelid),
                            COALESCE(i.indisprimary, false)
                     FROM pg_attribute a
                     JOIN pg_class c ON c.oid = a.attrelid
                     JOIN pg_namespace n ON n.oid = c.relnamespace
                     LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
                     LEFT JOIN pg_index i ON i.indrelid = a.attrelid
                          AND a.attnum = ANY(i.indkey) AND i.indisprimary
                     WHERE c.relname = {table} AND n.nspname = {schema_sql}
                       AND a.attnum > 0 AND NOT a.attisdropped
                     ORDER BY a.attnum",
                    table = table_name
                );
                let rows = self.pg_rows(&sql)?;
                let mut out = Vec::new();
                for row in rows {
                    out.push(ColumnMeta {
                        name: row.first().cloned().flatten().unwrap_or_default(),
                        decl_type: row.get(1).cloned().flatten().unwrap_or_default(),
                        not_null: row
                            .get(2)
                            .and_then(|v| v.as_deref())
                            .map(|v| v == "t" || v.eq_ignore_ascii_case("true"))
                            .unwrap_or(false),
                        default_value: row.get(3).cloned().flatten(),
                        primary_key: row
                            .get(4)
                            .and_then(|v| v.as_deref())
                            .map(|v| v == "t" || v.eq_ignore_ascii_case("true"))
                            .unwrap_or(false),
                        options: Vec::new(),
                    });
                }
                // Enum labels for any column backed by a PostgreSQL enum type.
                let enum_sql = format!(
                    "SELECT a.attname, e.enumlabel
                     FROM pg_attribute a
                     JOIN pg_class c ON c.oid = a.attrelid
                     JOIN pg_namespace n ON n.oid = c.relnamespace
                     JOIN pg_type t ON t.oid = a.atttypid
                     JOIN pg_enum e ON e.enumtypid = t.oid
                     WHERE c.relname = {table} AND n.nspname = {schema_sql}
                       AND a.attnum > 0 AND NOT a.attisdropped
                     ORDER BY a.attnum, e.enumsortorder",
                    table = table_name
                );
                if let Ok(enum_rows) = self.pg_rows(&enum_sql) {
                    for row in enum_rows {
                        let Some(name) = row.first().cloned().flatten() else {
                            continue;
                        };
                        let Some(label) = row.get(1).cloned().flatten() else {
                            continue;
                        };
                        if let Some(column) = out.iter_mut().find(|c| c.name == name) {
                            column.options.push(label);
                        }
                    }
                }
                Ok(out)
            }
        }
    }

    pub fn table_indexes(&self, table: &str) -> Vec<IndexMeta> {
        match &self.backend {
            Backend::Sqlite(conn) => table_indexes(conn, table),
            Backend::MySql(_) => {
                let mut conn = self.mysql();
                let rows: Vec<MyRow> = match conn.exec(
                    "SELECT index_name, non_unique, column_name
                     FROM information_schema.statistics
                     WHERE table_schema = DATABASE() AND table_name = ?
                     ORDER BY index_name, seq_in_index",
                    (table,),
                ) {
                    Ok(rows) => rows,
                    Err(_) => return Vec::new(),
                };
                let mut out: Vec<IndexMeta> = Vec::new();
                for row in rows {
                    let name = mysql_string(&row, 0).unwrap_or_default();
                    let non_unique = mysql_i64(&row, 1).unwrap_or(1);
                    let column = mysql_string(&row, 2).unwrap_or_default();
                    if let Some(existing) = out.iter_mut().find(|i| i.name == name) {
                        existing.columns.push(column);
                    } else {
                        out.push(IndexMeta {
                            name,
                            unique: non_unique == 0,
                            columns: vec![column],
                        });
                    }
                }
                out
            }
            Backend::Postgres(_) => {
                let (table_name, schema_sql) = pg_relation_predicate(table);
                let sql = format!(
                    "SELECT i.relname, ix.indisunique, a.attname
                     FROM pg_index ix
                     JOIN pg_class i ON i.oid = ix.indexrelid
                     JOIN pg_class t ON t.oid = ix.indrelid
                     JOIN pg_namespace n ON n.oid = t.relnamespace
                     JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = ANY(ix.indkey)
                     WHERE t.relname = {table} AND n.nspname = {schema_sql}
                     ORDER BY i.relname",
                    table = table_name
                );
                let mut out: Vec<IndexMeta> = Vec::new();
                if let Ok(rows) = self.pg_rows(&sql) {
                    for row in rows {
                        let name = row.first().cloned().flatten().unwrap_or_default();
                        let unique = row
                            .get(1)
                            .and_then(|v| v.as_deref())
                            .map(|v| v == "t" || v.eq_ignore_ascii_case("true"))
                            .unwrap_or(false);
                        let column = row.get(2).cloned().flatten().unwrap_or_default();
                        if let Some(existing) = out.iter_mut().find(|i| i.name == name) {
                            existing.columns.push(column);
                        } else {
                            out.push(IndexMeta {
                                name,
                                unique,
                                columns: vec![column],
                            });
                        }
                    }
                }
                out
            }
        }
    }

    pub fn table_ddl(&self, table: &str) -> Option<String> {
        match &self.backend {
            Backend::Sqlite(conn) => table_ddl(conn, table),
            Backend::MySql(_) => {
                let mut conn = self.mysql();
                let sql = format!("SHOW CREATE TABLE {}", self.quote(table));
                let rows: Vec<MyRow> = conn.query(sql).ok()?;
                rows.first().and_then(|row| mysql_string(row, 1))
            }
            Backend::Postgres(_) => {
                let columns = self.table_structure(table).ok()?;
                if columns.is_empty() {
                    return None;
                }
                let mut parts = Vec::new();
                for column in &columns {
                    let mut line = format!("    {} {}", self.quote(&column.name), column.decl_type);
                    if column.not_null {
                        line.push_str(" NOT NULL");
                    }
                    if let Some(default) = &column.default_value {
                        line.push_str(&format!(" DEFAULT {default}"));
                    }
                    if column.primary_key {
                        line.push_str(" PRIMARY KEY");
                    }
                    parts.push(line);
                }
                Some(format!(
                    "CREATE TABLE {} (\n{}\n);",
                    self.quote(table),
                    parts.join(",\n")
                ))
            }
        }
    }

    pub fn run_sql(&self, sql: &str, max_rows: usize) -> QueryOutcome {
        let start = Instant::now();
        if self.is_read_only() && statement_writes(sql) {
            return QueryOutcome {
                error: Some(
                    "Read-only mode is on: this statement would modify the database."
                        .to_string(),
                ),
                elapsed_ms: start.elapsed().as_millis(),
                ..QueryOutcome::default()
            };
        }
        let mut outcome = QueryOutcome::default();
        let statements = split_statements(sql);
        if statements.is_empty() {
            outcome.error = Some("There is no query to run.".to_string());
            outcome.elapsed_ms = start.elapsed().as_millis();
            return outcome;
        }
        for statement in statements {
            if outcome.error.is_some() {
                break;
            }
            if let Err(err) = self.run_statement(&statement, max_rows, &mut outcome) {
                outcome.error = Some(err);
            }
        }
        outcome.elapsed_ms = start.elapsed().as_millis();
        if outcome.error.is_none() && outcome.truncated {
            outcome.message = format!("Showing the first {max_rows} rows.");
        }
        outcome
    }

    fn run_statement(
        &self,
        sql: &str,
        max_rows: usize,
        outcome: &mut QueryOutcome,
    ) -> Result<(), String> {
        match &self.backend {
            Backend::Sqlite(conn) => run_single(conn, sql, max_rows, outcome),
            Backend::MySql(_) => {
                let mut conn = self.mysql();
                let mut result = conn.query_iter(sql).map_err(|e| e.to_string())?;
                let names: Vec<String> = {
                    let columns = result.columns();
                    columns
                        .as_ref()
                        .iter()
                        .map(|c| c.name_str().to_string())
                        .collect()
                };
                outcome.statements += 1;
                if names.is_empty() {
                    let affected = result.affected_rows() as usize;
                    outcome.rows_affected += affected;
                    while result.next().is_some() {}
                    return Ok(());
                }
                if outcome.columns.is_empty() {
                    outcome.columns = names;
                } else if outcome.columns != names {
                    outcome.columns = names;
                    outcome.rows.clear();
                }
                while let Some(row) = result.next() {
                    let row = row.map_err(|e| e.to_string())?;
                    if max_rows > 0 && outcome.rows.len() >= max_rows {
                        outcome.truncated = true;
                        continue;
                    }
                    let mut values = Vec::with_capacity(outcome.columns.len());
                    for i in 0..outcome.columns.len() {
                        values.push(row.as_ref(i).and_then(mysql_cell));
                    }
                    outcome.rows.push(values);
                }
                Ok(())
            }
            Backend::Postgres(_) => {
                let mut client = self.pg();
                let messages = client.simple_query(sql).map_err(|e| e.to_string())?;
                outcome.statements += 1;
                for message in messages {
                    match message {
                        postgres::SimpleQueryMessage::RowDescription(columns) => {
                            if outcome.columns.is_empty() && !columns.is_empty() {
                                outcome.columns =
                                    columns.iter().map(|c| c.name().to_string()).collect();
                            }
                        }
                        postgres::SimpleQueryMessage::Row(row) => {
                            if outcome.columns.is_empty() {
                                outcome.columns =
                                    row.columns().iter().map(|c| c.name().to_string()).collect();
                            }
                            if max_rows > 0 && outcome.rows.len() >= max_rows {
                                outcome.truncated = true;
                                continue;
                            }
                            let mut values = Vec::with_capacity(outcome.columns.len());
                            for i in 0..row.len() {
                                values.push(row.try_get(i).ok().flatten().map(str::to_string));
                            }
                            outcome.rows.push(values);
                        }
                        postgres::SimpleQueryMessage::CommandComplete(affected) => {
                            if outcome.columns.is_empty() {
                                outcome.rows_affected += affected as usize;
                            }
                        }
                        _ => {}
                    }
                }
                Ok(())
            }
        }
    }

    pub fn fetch_table(
        &self,
        table: &str,
        limit: usize,
        offset: usize,
        filter: &TableFilter,
        sort: &[SortSpec],
    ) -> TablePage {
        match &self.backend {
            Backend::Sqlite(conn) => fetch_table(conn, table, limit, offset, filter, sort),
            Backend::MySql(_) => self.mysql_fetch_table(table, limit, offset, filter, sort),
            Backend::Postgres(_) => self.pg_fetch_table(table, limit, offset, filter, sort),
        }
    }

    fn mysql_fetch_table(
        &self,
        table: &str,
        limit: usize,
        offset: usize,
        filter: &TableFilter,
        sort: &[SortSpec],
    ) -> TablePage {
        let start = Instant::now();
        let mut out = TablePage::default();
        let structure = match self.table_structure(table) {
            Ok(columns) => columns,
            Err(e) => {
                out.error = Some(e);
                return out;
            }
        };
        if structure.is_empty() {
            out.error = Some("This table has no columns.".to_string());
            out.elapsed_ms = start.elapsed().as_millis();
            return out;
        }
        let names: Vec<String> = structure.iter().map(|c| c.name.clone()).collect();
        let primary_key = structure
            .iter()
            .find(|c| c.primary_key && is_integer_type(&c.decl_type))
            .map(|c| c.name.clone());

        let (where_sql, params) = build_where(filter, &names, Dialect::MySql);

        let mut conn = self.mysql();
        let count_sql = format!("SELECT COUNT(*) FROM {} {}", self.quote(table), where_sql);
        let total: i64 = if params.is_empty() {
            conn.query_first::<i64, _>(count_sql)
                .ok()
                .flatten()
                .unwrap_or(0)
        } else {
            conn.exec_first::<i64, _, _>(count_sql, params.clone())
                .ok()
                .flatten()
                .unwrap_or(0)
        };

        let select_columns = if let Some(key) = &primary_key {
            // The table-qualified star is required: MySQL rejects a bare `*`
            // after an expression in the select list.
            format!(
                "CAST({} AS SIGNED) AS __rowkey, {}.*",
                self.quote(key),
                self.quote(table)
            )
        } else {
            "*".to_string()
        };
        let default_order = primary_key
            .as_ref()
            .map(|k| self.quote(k))
            .unwrap_or_else(|| "1".to_string());
        let order = build_order_by(sort, &names, Dialect::MySql);
        let order = if order.is_empty() {
            format!("ORDER BY {default_order}")
        } else {
            format!("{order}, {default_order}")
        };
        // MySQL has no "no limit" keyword; its documented idiom is the
        // largest unsigned BIGINT value, which OFFSET still accepts.
        let limit_sql = if limit == 0 {
            "18446744073709551615".to_string()
        } else {
            limit.to_string()
        };
        let sql = format!(
            "SELECT {} FROM {} {} {} LIMIT {} OFFSET {}",
            select_columns,
            self.quote(table),
            where_sql,
            order,
            limit_sql,
            offset
        );
        let result = if params.is_empty() {
            conn.query::<MyRow, _>(&sql)
        } else {
            conn.exec::<MyRow, _, _>(&sql, params)
        };

        out.total = total;
        out.columns = names.clone();
        out.key_column = primary_key;
        out.editable = out.key_column.is_some();
        match result {
            Ok(rows) => {
                let skip = if out.key_column.is_some() { 1 } else { 0 };
                for row in rows {
                    let key = if skip == 1 {
                        mysql_i64(&row, 0).map(|v| v.to_string())
                    } else {
                        None
                    };
                    let mut values = Vec::with_capacity(names.len());
                    for i in 0..names.len() {
                        values.push(row.as_ref(i + skip).and_then(mysql_cell));
                    }
                    out.rows.push(values);
                    out.rowids.push(key);
                }
            }
            Err(e) => {
                out.error = Some(e.to_string());
                out.rows.clear();
                out.rowids.clear();
            }
        }
        out.elapsed_ms = start.elapsed().as_millis();
        out
    }

    fn pg_fetch_table(
        &self,
        table: &str,
        limit: usize,
        offset: usize,
        filter: &TableFilter,
        sort: &[SortSpec],
    ) -> TablePage {
        let start = Instant::now();
        let mut out = TablePage::default();
        let structure = match self.table_structure(table) {
            Ok(columns) => columns,
            Err(e) => {
                out.error = Some(e);
                return out;
            }
        };
        if structure.is_empty() {
            out.error = Some("This table has no columns.".to_string());
            out.elapsed_ms = start.elapsed().as_millis();
            return out;
        }
        let names: Vec<String> = structure.iter().map(|c| c.name.clone()).collect();
        // `pg_rows` runs literal SQL, so the builder escapes the values.
        let (where_sql, _) = build_where(filter, &names, Dialect::Postgres);
        let limit_sql = if limit == 0 {
            String::new()
        } else {
            format!("LIMIT {} ", limit)
        };
        let count_sql = format!("SELECT COUNT(*) FROM {} {}", self.quote(table), where_sql);
        out.total = self
            .pg_rows(&count_sql)
            .ok()
            .and_then(|rows| rows.into_iter().next())
            .and_then(|row| row.into_iter().next().flatten())
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0);

        let sort_order = build_order_by(sort, &names, Dialect::Postgres);
        let order = if sort_order.is_empty() {
            "ORDER BY ctid".to_string()
        } else {
            format!("{sort_order}, ctid")
        };
        let fallback_order = sort_order;
        let sql = format!(
            "SELECT ctid::text AS __rowkey, * FROM {} {} {} {}OFFSET {}",
            self.quote(table),
            where_sql,
            order,
            limit_sql,
            offset
        );
        match self.pg_rows(&sql) {
            Ok(rows) => {
                out.columns = names.clone();
                out.key_column = Some("ctid".to_string());
                out.editable = true;
                for row in rows {
                    let key = row.first().cloned().flatten();
                    let values = row.into_iter().skip(1).collect();
                    out.rows.push(values);
                    out.rowids.push(key);
                }
            }
            Err(_) => {
                // Views and some partitioned relations have no ctid: browse
                // them read-only instead of failing.
                let fallback = format!(
                    "SELECT * FROM {} {} {} {}OFFSET {}",
                    self.quote(table),
                    where_sql,
                    fallback_order,
                    limit_sql,
                    offset
                );
                match self.pg_rows(&fallback) {
                    Ok(rows) => {
                        out.columns = names;
                        out.key_column = None;
                        out.editable = false;
                        out.rows = rows;
                        out.rowids = vec![None; out.rows.len()];
                    }
                    Err(e) => {
                        out.error = Some(e);
                        out.rows.clear();
                        out.rowids.clear();
                    }
                }
            }
        }
        out.elapsed_ms = start.elapsed().as_millis();
        out
    }

    /// Updates one cell. `None` writes SQL NULL and `Some("")` writes an empty
    /// string, so the grid can offer the same NULL/empty distinction as the
    /// row detail panel. This is a thin alias of [`Db::update_field`].
    pub fn update_cell(
        &self,
        table: &str,
        column: &str,
        key_column: &str,
        key_value: &str,
        value: Option<&str>,
    ) -> Result<usize, String> {
        self.update_field(table, column, key_column, key_value, value)
    }

    /// Updates one field of one record, addressed exactly like `update_cell`.
    /// `None` writes SQL NULL; `Some("")` writes an empty string. The row
    /// detail panel uses this for its per-field vertical editor.
    pub fn update_field(
        &self,
        table: &str,
        column: &str,
        key_column: &str,
        key_value: &str,
        value: Option<&str>,
    ) -> Result<usize, String> {
        self.ensure_writable("update rows")?;
        match &self.backend {
            Backend::Sqlite(conn) => {
                let key_clause = if key_column == "rowid" {
                    "rowid = ?2".to_string()
                } else {
                    format!("{} = ?2", quote_ident(key_column))
                };
                let sql = format!(
                    "UPDATE {} SET {} = ?1 WHERE {}",
                    quote_ident(table),
                    quote_ident(column),
                    key_clause
                );
                let affected = if let Ok(number) = key_value.parse::<i64>() {
                    conn.execute(&sql, rusqlite::params![value, number])
                } else {
                    conn.execute(&sql, rusqlite::params![value, key_value])
                }
                .map_err(|e| e.to_string())?;
                Ok(affected)
            }
            Backend::MySql(_) => {
                let sql = format!(
                    "UPDATE {} SET {} = ? WHERE {} = ?",
                    self.quote(table),
                    self.quote(column),
                    self.quote(key_column)
                );
                let mut conn = self.mysql();
                conn.exec_drop(sql, (value, key_value))
                    .map_err(|e| e.to_string())?;
                Ok(conn.affected_rows() as usize)
            }
            Backend::Postgres(_) => {
                let sql = format!(
                    "UPDATE {} SET {} = $1 WHERE ctid = $2::tid",
                    self.quote(table),
                    self.quote(column)
                );
                let mut client = self.pg();
                let affected = client
                    .execute(&sql, &[&value, &key_value])
                    .map_err(|e| e.to_string())?;
                Ok(affected as usize)
            }
        }
    }

    /// Deletes one record identified the same way cell editing does: an
    /// integer primary key, `rowid` on SQLite, or `ctid` on PostgreSQL.
    pub fn delete_row(
        &self,
        table: &str,
        key_column: &str,
        key_value: &str,
    ) -> Result<usize, String> {
        self.ensure_writable("delete rows")?;
        match &self.backend {
            Backend::Sqlite(conn) => {
                let key_clause = if key_column == "rowid" {
                    "rowid = ?1".to_string()
                } else {
                    format!("{} = ?1", quote_ident(key_column))
                };
                let sql = format!("DELETE FROM {} WHERE {}", quote_ident(table), key_clause);
                let affected = if let Ok(number) = key_value.parse::<i64>() {
                    conn.execute(&sql, rusqlite::params![number])
                } else {
                    conn.execute(&sql, rusqlite::params![key_value])
                }
                .map_err(|e| e.to_string())?;
                Ok(affected)
            }
            Backend::MySql(_) => {
                let sql = format!(
                    "DELETE FROM {} WHERE {} = ?",
                    self.quote(table),
                    self.quote(key_column)
                );
                let mut conn = self.mysql();
                conn.exec_drop(sql, (key_value,))
                    .map_err(|e| e.to_string())?;
                Ok(conn.affected_rows() as usize)
            }
            Backend::Postgres(_) => {
                let sql = format!("DELETE FROM {} WHERE ctid = $1::tid", self.quote(table));
                let mut client = self.pg();
                let affected = client
                    .execute(&sql, &[&key_value])
                    .map_err(|e| e.to_string())?;
                Ok(affected as usize)
            }
        }
    }

    /// Reads every row of a table for an export. Paging is skipped on purpose:
    /// an export contains every row the filter matches, not just the visible
    /// page.
    fn export_rows(
        &self,
        table: &str,
        filter: &TableFilter,
        sort: &[SortSpec],
    ) -> Result<(Vec<String>, Vec<Vec<Option<String>>>), String> {
        let page = self.fetch_table(table, 0, 0, filter, sort);
        match page.error {
            Some(error) => Err(error),
            None => Ok((page.columns, page.rows)),
        }
    }

    /// Table data as a UTF-8 CSV file (with a BOM so Excel detects UTF-8).
    pub fn export_table_csv(
        &self,
        table: &str,
        filter: &TableFilter,
        sort: &[SortSpec],
    ) -> Result<Vec<u8>, String> {
        let (columns, rows) = self.export_rows(table, filter, sort)?;
        Ok(csv_bytes(&columns, &rows))
    }

    /// Table data as an Excel `.xlsx` workbook.
    pub fn export_table_xlsx(
        &self,
        table: &str,
        filter: &TableFilter,
        sort: &[SortSpec],
    ) -> Result<Vec<u8>, String> {
        let (columns, rows) = self.export_rows(table, filter, sort)?;
        Ok(crate::xlsx::workbook(&table_display_name(table), &columns, &rows))
    }

    /// Copies a table's structure and rows into a new table. SQLite keeps the
    /// original `CREATE TABLE` text with only the name replaced, so primary
    /// keys and constraints survive; MySQL/PostgreSQL use their native
    /// `LIKE` clones. Secondary indexes are not copied.
    pub fn clone_table(&self, source: &str, target: &str) -> Result<(), String> {
        self.ensure_writable("clone tables")?;
        let target = target.trim();
        if target.is_empty() {
            return Err("Give the new table a name.".to_string());
        }
        let (schema, source_name) = split_table_id(source);
        if target.eq_ignore_ascii_case(source_name) {
            return Err("Choose a different name for the copy.".to_string());
        }
        // A clone stays in the same schema as its source; without this, a
        // PostgreSQL table outside `search_path` would be copied into the
        // current schema instead.
        let target_id = TableId::with_schema(schema.map(str::to_string), target);
        if self
            .list_tables()?
            .iter()
            .any(|table| table.id() == target_id)
        {
            return Err(format!("A table or view named {target} already exists."));
        }
        let target_sql = target_id.sql();
        match &self.backend {
            Backend::Sqlite(conn) => {
                let display = table_display_name(source);
                let ddl = self.table_ddl(source).ok_or_else(|| {
                    format!("No CREATE statement is available for {display}.")
                })?;
                let create = rewrite_create_table(&ddl, target)?;
                let create = create.trim().trim_end_matches(';');
                let script = format!(
                    "{create};\nINSERT INTO {} SELECT * FROM {};",
                    self.quote(&target_sql),
                    self.quote(source)
                );
                conn.execute_batch(&script).map_err(|e| e.to_string())
            }
            Backend::MySql(_) => {
                let sql = format!(
                    "CREATE TABLE {} LIKE {};\nINSERT INTO {} SELECT * FROM {};",
                    self.quote(&target_sql),
                    self.quote(source),
                    self.quote(&target_sql),
                    self.quote(source)
                );
                self.execute(&sql)
            }
            Backend::Postgres(_) => {
                let sql = format!(
                    "CREATE TABLE {} (LIKE {} INCLUDING ALL);\nINSERT INTO {} SELECT * FROM {};",
                    self.quote(&target_sql),
                    self.quote(source),
                    self.quote(&target_sql),
                    self.quote(source)
                );
                self.execute(&sql)
            }
        }
    }

    /// Removes every row but keeps the table. SQLite has no `TRUNCATE`, so it
    /// uses `DELETE` and resets the `sqlite_sequence` counter when present.
    pub fn truncate_table(&self, table: &str) -> Result<(), String> {
        self.ensure_writable("truncate tables")?;
        match &self.backend {
            Backend::Sqlite(conn) => {
                conn.execute(&format!("DELETE FROM {}", quote_ident(table)), [])
                    .map_err(|e| e.to_string())?;
                let _ = conn.execute(
                    "DELETE FROM sqlite_sequence WHERE name = ?1",
                    rusqlite::params![split_table_id(table).1],
                );
                Ok(())
            }
            Backend::MySql(_) => {
                self.execute(&format!("TRUNCATE TABLE {}", self.quote(table)))
            }
            Backend::Postgres(_) => {
                self.execute(&format!("TRUNCATE TABLE {}", self.quote(table)))
            }
        }
    }

    /// Drops a table and all of its rows.
    pub fn drop_table(&self, table: &str) -> Result<(), String> {
        self.ensure_writable("drop tables")?;
        match &self.backend {
            Backend::Sqlite(conn) => conn
                .execute_batch(&format!("DROP TABLE {}", quote_ident(table)))
                .map_err(|e| e.to_string()),
            Backend::MySql(_) => {
                self.execute(&format!("DROP TABLE {}", self.quote(table)))
            }
            Backend::Postgres(_) => {
                self.execute(&format!("DROP TABLE {}", self.quote(table)))
            }
        }
    }

    /// Runs one or more statements and reports only whether they succeeded.
    pub fn execute(&self, sql: &str) -> Result<(), String> {
        let outcome = self.run_sql(sql, 0);
        match outcome.error {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    // ------------------------------------------------------------------
    // SQL dumps, imports and backups
    // ------------------------------------------------------------------

    /// SQL dump of one table: its `CREATE` statement followed by one `INSERT`
    /// per row. `filter` limits the rows exactly like the table view. Values
    /// are quoted by the database itself (`quote()`, `QUOTE()` and
    /// `quote_nullable()`), so quotes, backslashes, NULLs and blobs survive the
    /// round trip.
    pub fn dump_table_sql(
        &self,
        table: &str,
        filter: &TableFilter,
        sort: &[SortSpec],
    ) -> Result<String, String> {
        let display = table_display_name(table);
        let structure = self.table_structure(table)?;
        if structure.is_empty() {
            return Err(format!("{display} has no columns."));
        }
        let columns: Vec<String> = structure.iter().map(|c| c.name.clone()).collect();
        let ddl = self
            .table_ddl(table)
            .ok_or_else(|| format!("No CREATE statement is available for {display}."))?;
        let select = self.dump_select(table, &columns, filter, sort);
        let outcome = self.run_sql(&select, 0);
        if let Some(error) = outcome.error {
            return Err(error);
        }
        let mut out = format!("-- Table: {display}\n");
        if !filter.is_empty() {
            out.push_str("-- Filtered export: only the rows matching the active filter.\n");
        }
        out.push_str(&with_terminator(&ddl));
        out.push_str("\n\n");
        let quoted_columns: Vec<String> = columns.iter().map(|c| self.quote(c)).collect();
        for row in &outcome.rows {
            let values: Vec<&str> = row.iter().map(|v| v.as_deref().unwrap_or("NULL")).collect();
            out.push_str(&format!(
                "INSERT INTO {} ({}) VALUES ({});\n",
                self.quote(table),
                quoted_columns.join(", "),
                values.join(", ")
            ));
        }
        out.push('\n');
        Ok(out)
    }

    /// SQL dump of the whole database: every table with its data, then the
    /// views that expose a `CREATE` statement.
    pub fn dump_database_sql(&self, name: &str) -> Result<String, String> {
        let tables = self.list_tables()?;
        if tables.is_empty() {
            return Err("This database has no tables to export.".to_string());
        }
        let mut out = format!(
            "-- Dataloom SQL dump\n-- Database: {}\n-- Engine: {}\n-- Date: {}\n\n",
            name,
            self.driver.label(),
            utc_stamp(),
        );
        for table in tables.iter().filter(|t| !t.is_view) {
            out.push_str(&self.dump_table_sql(
                &table.id().sql(),
                &TableFilter::default(),
                &[],
            )?);
        }
        for view in tables.iter().filter(|t| t.is_view) {
            if let Some(ddl) = self.table_ddl(&view.id().sql()) {
                out.push_str(&format!(
                    "-- View: {}\n{}\n\n",
                    view.id().display(),
                    with_terminator(&ddl)
                ));
            }
        }
        Ok(out)
    }

    /// Runs an SQL script (an imported dump), one statement at a time.
    pub fn import_sql(&self, script: &str) -> Result<usize, String> {
        self.ensure_writable("run a SQL import")?;
        let statements = split_statements(script);
        if statements.is_empty() {
            return Err("The file contains no SQL statements.".to_string());
        }
        for statement in &statements {
            self.execute(statement)?;
        }
        Ok(statements.len())
    }

    /// Creates a backup of the connected database: a byte-for-byte copy for
    /// SQLite and a SQL dump for the server engines. Returns the new file.
    pub fn create_backup(&self, saved: &SavedConnection) -> Result<PathBuf, String> {
        create_backup_in(&backups_dir(saved), saved, self)
    }

    /// Row values as SQL literals, ready to be pasted into an INSERT. The
    /// filter is rendered with inline literals because the dump is one SQL
    /// script, not a parameterized query.
    fn dump_select(
        &self,
        table: &str,
        columns: &[String],
        filter: &TableFilter,
        sort: &[SortSpec],
    ) -> String {
        let exprs: Vec<String> = columns
            .iter()
            .map(|column| match self.driver {
                Driver::Sqlite => format!("quote({})", self.quote(column)),
                Driver::MySql => format!("QUOTE({})", self.quote(column)),
                Driver::Postgres => {
                    format!("quote_nullable(CAST({} AS TEXT))", self.quote(column))
                }
            })
            .collect();
        let where_sql = build_where_literals(filter, columns, self.dialect());
        let order_sql = build_order_by(sort, columns, self.dialect());
        format!(
            "SELECT {} FROM {} {} {}",
            exprs.join(", "),
            self.quote(table),
            where_sql,
            order_sql
        )
    }

    fn dialect(&self) -> Dialect {
        match self.driver {
            Driver::Sqlite => Dialect::Sqlite,
            Driver::MySql => Dialect::MySql,
            Driver::Postgres => Dialect::Postgres,
        }
    }

    /// Inserts a record. Empty values are omitted by the caller so the column
    /// default (or NULL) applies; this keeps auto-increment keys working.
    /// Values are bound as parameters so quotes and backslashes are safe.
    pub fn insert_row(&self, table: &str, values: &[(String, String)]) -> Result<(), String> {
        self.ensure_writable("insert rows")?;
        let columns: Vec<String> = values.iter().map(|(c, _)| self.quote(c)).collect();
        match &self.backend {
            Backend::Sqlite(conn) => {
                if values.is_empty() {
                    conn.execute(
                        &format!("INSERT INTO {} DEFAULT VALUES", quote_ident(table)),
                        [],
                    )
                    .map_err(|e| e.to_string())?;
                } else {
                    let placeholders: Vec<String> =
                        (1..=values.len()).map(|i| format!("?{i}")).collect();
                    let sql = format!(
                        "INSERT INTO {} ({}) VALUES ({})",
                        quote_ident(table),
                        columns.join(", "),
                        placeholders.join(", ")
                    );
                    let params: Vec<&dyn rusqlite::ToSql> = values
                        .iter()
                        .map(|(_, v)| v as &dyn rusqlite::ToSql)
                        .collect();
                    conn.execute(&sql, params.as_slice())
                        .map_err(|e| e.to_string())?;
                }
                Ok(())
            }
            Backend::MySql(_) => {
                let sql = if values.is_empty() {
                    format!("INSERT INTO {} () VALUES ()", self.quote(table))
                } else {
                    let placeholders = vec!["?"; values.len()].join(", ");
                    format!(
                        "INSERT INTO {} ({}) VALUES ({})",
                        self.quote(table),
                        columns.join(", "),
                        placeholders
                    )
                };
                let params = mysql::params::Params::Positional(
                    values
                        .iter()
                        .map(|(_, v)| MyValue::Bytes(v.as_bytes().to_vec()))
                        .collect(),
                );
                let mut conn = self.mysql();
                conn.exec_drop(sql, params).map_err(|e| e.to_string())
            }
            Backend::Postgres(_) => {
                let sql = if values.is_empty() {
                    format!("INSERT INTO {} DEFAULT VALUES", self.quote(table))
                } else {
                    let placeholders: Vec<String> =
                        (1..=values.len()).map(|i| format!("${i}")).collect();
                    format!(
                        "INSERT INTO {} ({}) VALUES ({})",
                        self.quote(table),
                        columns.join(", "),
                        placeholders.join(", ")
                    )
                };
                let params: Vec<&(dyn postgres::types::ToSql + Sync)> = values
                    .iter()
                    .map(|(_, v)| v as &(dyn postgres::types::ToSql + Sync))
                    .collect();
                let mut client = self.pg();
                client
                    .execute(sql.as_str(), params.as_slice())
                    .map_err(|e| e.to_string())?;
                Ok(())
            }
        }
    }

    /// Adds a column. `decl_type` is passed through as written because each
    /// engine spells its types differently.
    pub fn add_column(
        &self,
        table: &str,
        column: &str,
        decl_type: &str,
        not_null: bool,
        default_value: &str,
    ) -> Result<(), String> {
        let mut sql = format!(
            "ALTER TABLE {} ADD COLUMN {}",
            self.quote(table),
            self.quote(column)
        );
        let kind = decl_type.trim();
        if !kind.is_empty() {
            sql.push(' ');
            sql.push_str(kind);
        }
        if not_null {
            sql.push_str(" NOT NULL");
        }
        let default = default_value.trim();
        if !default.is_empty() {
            sql.push_str(" DEFAULT ");
            sql.push_str(&default_expr(default));
        }
        self.execute(&sql)
    }

    pub fn rename_column(&self, table: &str, old: &str, new: &str) -> Result<(), String> {
        self.ensure_writable("rename columns")?;
        if old == new {
            return Ok(());
        }
        let sql = format!(
            "ALTER TABLE {} RENAME COLUMN {} TO {}",
            self.quote(table),
            self.quote(old),
            self.quote(new)
        );
        self.execute(&sql)
    }

    /// Changes a column's type and nullability. SQLite needs a full table
    /// rebuild for this, so it only supports renaming here.
    pub fn change_column(
        &self,
        table: &str,
        column: &str,
        decl_type: &str,
        not_null: bool,
    ) -> Result<(), String> {
        self.ensure_writable("change columns")?;
        match self.driver {
            Driver::Sqlite => Err(
                "SQLite can rename a column, but changing its type or nullability needs a table rebuild."
                    .to_string(),
            ),
            Driver::MySql => {
                let mut sql = format!(
                    "ALTER TABLE {} MODIFY COLUMN {} {}",
                    self.quote(table),
                    self.quote(column),
                    decl_type.trim()
                );
                sql.push_str(if not_null { " NOT NULL" } else { " NULL" });
                self.execute(&sql)
            }
            Driver::Postgres => {
                let mut sql = format!(
                    "ALTER TABLE {} ALTER COLUMN {} TYPE {}; ALTER TABLE {} ALTER COLUMN {} {}",
                    self.quote(table),
                    self.quote(column),
                    decl_type.trim(),
                    self.quote(table),
                    self.quote(column),
                    if not_null { "SET NOT NULL" } else { "DROP NOT NULL" }
                );
                sql.push(';');
                self.execute(&sql)
            }
        }
    }

    pub fn drop_column(&self, table: &str, column: &str) -> Result<(), String> {
        self.ensure_writable("drop columns")?;
        let sql = format!(
            "ALTER TABLE {} DROP COLUMN {}",
            self.quote(table),
            self.quote(column)
        );
        self.execute(&sql)
    }

    pub fn default_query(&self) -> String {
        match self.driver {
            Driver::Sqlite => default_query(),
            Driver::MySql => "-- MySQL workspace\n-- Press Command+Enter to run the current query.\n\nSHOW TABLES;".to_string(),
            Driver::Postgres => "-- PostgreSQL workspace\n-- Press Command+Enter to run the current query.\n\nSELECT table_schema, table_name\nFROM information_schema.tables\nWHERE table_schema NOT IN ('pg_catalog', 'information_schema')\nORDER BY 1, 2;".to_string(),
        }
    }

    fn pg_rows(&self, sql: &str) -> Result<Vec<Vec<Option<String>>>, String> {
        let mut client = self.pg();
        let messages = client.simple_query(sql).map_err(|e| e.to_string())?;
        let mut rows = Vec::new();
        for message in messages {
            if let postgres::SimpleQueryMessage::Row(row) = message {
                let mut values = Vec::with_capacity(row.len());
                for i in 0..row.len() {
                    values.push(row.try_get(i).ok().flatten().map(str::to_string));
                }
                rows.push(values);
            }
        }
        Ok(rows)
    }
}

fn is_integer_type(decl_type: &str) -> bool {
    let lower = decl_type.to_ascii_lowercase();
    lower.contains("int") || lower.contains("serial") || lower == "year"
}

/// Reads a MySQL cell without panicking when the value is NULL or has a
/// different type. `Row::get` panics on conversion errors, which would close
/// the whole app when a column has no default value.
fn mysql_string(row: &MyRow, index: usize) -> Option<String> {
    row.get_opt::<String, _>(index).and_then(|value| value.ok())
}

fn mysql_i64(row: &MyRow, index: usize) -> Option<i64> {
    row.get_opt::<i64, _>(index).and_then(|value| value.ok())
}

fn mysql_cell(value: &MyValue) -> Option<String> {
    match value {
        MyValue::NULL => None,
        MyValue::Bytes(bytes) => Some(String::from_utf8_lossy(bytes).to_string()),
        MyValue::Int(number) => Some(number.to_string()),
        MyValue::UInt(number) => Some(number.to_string()),
        MyValue::Float(number) => Some(number.to_string()),
        MyValue::Double(number) => Some(number.to_string()),
        MyValue::Date(year, month, day, hour, minute, second, micros) => {
            if *hour == 0 && *minute == 0 && *second == 0 && *micros == 0 {
                Some(format!("{year:04}-{month:02}-{day:02}"))
            } else {
                Some(format!(
                    "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}"
                ))
            }
        }
        MyValue::Time(negative, days, hours, minutes, seconds, _) => {
            let sign = if *negative { "-" } else { "" };
            let total_hours = *days as u64 * 24 + *hours as u64;
            Some(format!("{sign}{total_hours:02}:{minutes:02}:{seconds:02}"))
        }
    }
}

/// Quotes a string literal for PostgreSQL's simple-query protocol.
fn pg_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

// ---------------------------------------------------------------------------
// Tables and structure
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TableInfo {
    pub name: String,
    /// PostgreSQL schema the table lives in, when it is not the connection's
    /// current schema. SQLite and MySQL leave this empty.
    pub schema: Option<String>,
    pub is_view: bool,
    pub rows: Option<i64>,
}

impl TableInfo {
    pub fn id(&self) -> TableId {
        TableId::with_schema(self.schema.clone(), self.name.clone())
    }
}

/// Internal separator between schema and table in the string form passed to
/// the SQL builders. It never reaches the UI.
const TABLE_QUALIFIER: char = '\u{1f}';

/// A table addressed by an optional schema. PostgreSQL connections can expose
/// tables from several schemas; SQLite and MySQL normally leave `schema`
/// empty.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TableId {
    pub schema: Option<String>,
    pub name: String,
}

impl TableId {
    pub fn with_schema(schema: Option<String>, name: impl Into<String>) -> Self {
        TableId {
            schema,
            name: name.into(),
        }
    }

    /// Name shown in tabs, dialogs and the sidebar.
    pub fn display(&self) -> String {
        match &self.schema {
            Some(schema) if !schema.is_empty() => format!("{schema}.{}", self.name),
            _ => self.name.clone(),
        }
    }

    /// String handed to the SQL builders. It is deliberately not the display
    /// name: a table name may legally contain dots, so the schema and table are
    /// separated by a control character instead.
    pub fn sql(&self) -> String {
        match &self.schema {
            Some(schema) if !schema.is_empty() => {
                format!("{schema}{TABLE_QUALIFIER}{}", self.name)
            }
            _ => self.name.clone(),
        }
    }
}

impl std::fmt::Display for TableId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.display())
    }
}

fn split_table_id(table: &str) -> (Option<&str>, &str) {
    match table.split_once(TABLE_QUALIFIER) {
        Some((schema, name)) => (Some(schema), name),
        None => (None, table),
    }
}

/// SQL predicate parts for a PostgreSQL relation: the `relname` literal and
/// the schema expression (a literal for another schema, `current_schema()`
/// when the table is unqualified).
fn pg_relation_predicate(table: &str) -> (String, String) {
    let (schema, name) = split_table_id(table);
    let schema_sql = match schema {
        Some(schema) => pg_literal(schema),
        None => "current_schema()".to_string(),
    };
    (pg_literal(name), schema_sql)
}

/// Turns the internal SQL name back into the name shown to people.
pub fn table_display_name(table: &str) -> String {
    let (schema, name) = split_table_id(table);
    match schema {
        Some(schema) if !schema.is_empty() => format!("{schema}.{name}"),
        _ => name.to_string(),
    }
}

#[derive(Debug, Clone, Default)]
pub struct ColumnMeta {
    pub name: String,
    pub decl_type: String,
    pub not_null: bool,
    pub default_value: Option<String>,
    pub primary_key: bool,
    /// Values of an enum type/check constraint, when the column has a closed
    /// set of allowed values. Drives the dropdown editor.
    pub options: Vec<String>,
}

/// How a cell or row-detail field should be edited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldEditor {
    Text,
    Bool,
    Date,
    DateTime,
    Json,
    Blob,
    Enum(Vec<String>),
}

impl FieldEditor {
    /// Chooses the editor from the declared type plus any enum values. The
    /// declared types come straight from the server, so the matching is
    /// deliberately loose (`tinyint(1)` is MySQL's BOOLEAN).
    pub fn from_decl(decl_type: &str, options: &[String]) -> FieldEditor {
        if !options.is_empty() {
            return FieldEditor::Enum(options.to_vec());
        }
        let t = decl_type.trim().to_ascii_lowercase();
        if t == "bool" || t == "boolean" || t.starts_with("tinyint(1)") {
            FieldEditor::Bool
        } else if t.contains("timestamp") || t.contains("datetime") {
            FieldEditor::DateTime
        } else if t == "date" {
            FieldEditor::Date
        } else if t == "json" || t == "jsonb" {
            FieldEditor::Json
        } else if t.contains("blob")
            || t.contains("bytea")
            || t.contains("binary")
            || t.contains("image")
            || t.contains("bit(")
        {
            FieldEditor::Blob
        } else {
            FieldEditor::Text
        }
    }

    pub fn is_modal(&self) -> bool {
        matches!(self, FieldEditor::Json | FieldEditor::Blob)
    }

    pub fn label(&self) -> &'static str {
        match self {
            FieldEditor::Text => "text",
            FieldEditor::Bool => "boolean",
            FieldEditor::Date => "date",
            FieldEditor::DateTime => "datetime",
            FieldEditor::Json => "json",
            FieldEditor::Blob => "blob",
            FieldEditor::Enum(_) => "enum",
        }
    }
}

/// Parses the options out of a MySQL `enum('a','b')` column type.
fn parse_enum_type(decl_type: &str) -> Vec<String> {
    let lower = decl_type.to_ascii_lowercase();
    if !lower.starts_with("enum(") {
        return Vec::new();
    }
    let inner = decl_type[decl_type.find('(').map(|i| i + 1).unwrap_or(0)..]
        .trim_end_matches(')');
    let mut out = Vec::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            let mut value = String::new();
            while let Some(c) = chars.next() {
                if c == '\'' {
                    if chars.peek() == Some(&'\'') {
                        value.push('\'');
                        chars.next();
                    } else {
                        break;
                    }
                } else if c == '\\' {
                    if let Some(next) = chars.next() {
                        value.push(next);
                    }
                } else {
                    value.push(c);
                }
            }
            out.push(value);
        }
    }
    out
}

/// Best-effort extraction of `CHECK (column IN ('a','b'))` values from a
/// SQLite `CREATE TABLE` statement. SQLite has no enum type, so this is the
/// only closed set the app can infer without looking at every value.
fn sqlite_check_options(ddl: &str, column: &str) -> Vec<String> {
    let lower = ddl.to_ascii_lowercase();
    let column_lower = column.to_ascii_lowercase();
    let mut search = 0;
    while let Some(pos) = lower[search..].find("check") {
        let start = search + pos + "check".len();
        let Some(open) = lower[start..].find('(').map(|i| start + i) else {
            break;
        };
        let mut depth = 0i32;
        let mut end = open;
        for (i, c) in lower[open..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + i;
                        break;
                    }
                }
                _ => {}
            }
        }
        let clause_end = end.min(lower.len().saturating_sub(1));
        let clause = &lower[open..=clause_end];
        if clause.contains(&column_lower) {
            if let Some(in_pos) = clause.find(" in ") {
                let list = &clause[in_pos + 4..];
                if let Some(open_paren) = list.find('(') {
                    let values = &list[open_paren + 1..];
                    let mut out = Vec::new();
                    let mut chars = values.chars().peekable();
                    while let Some(c) = chars.next() {
                        if c == '\'' {
                            let mut value = String::new();
                            while let Some(c) = chars.next() {
                                if c == '\'' {
                                    if chars.peek() == Some(&'\'') {
                                        value.push('\'');
                                        chars.next();
                                    } else {
                                        break;
                                    }
                                } else {
                                    value.push(c);
                                }
                            }
                            out.push(value);
                        }
                    }
                    if !out.is_empty() {
                        return out;
                    }
                }
            }
        }
        search = clause_end.max(start) + 1;
        if search >= lower.len() {
            break;
        }
    }
    Vec::new()
}

#[derive(Debug, Clone, Default)]
pub struct IndexMeta {
    pub name: String,
    pub unique: bool,
    pub columns: Vec<String>,
}

pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// CSV with a UTF-8 BOM (Excel-friendly), CRLF rows, RFC 4180 quoting and an
/// empty field for SQL NULL.
pub fn csv_bytes(columns: &[String], rows: &[Vec<Option<String>>]) -> Vec<u8> {
    fn push_field(out: &mut String, value: Option<&str>) {
        let Some(value) = value else {
            return;
        };
        let quote = value.contains([',', '"', '\n', '\r'])
            || value.starts_with(char::is_whitespace)
            || value.ends_with(char::is_whitespace);
        if quote {
            out.push('"');
            out.push_str(&value.replace('"', "\"\""));
            out.push('"');
        } else {
            out.push_str(value);
        }
    }

    let mut text = String::new();
    for (index, column) in columns.iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        push_field(&mut text, Some(column));
    }
    text.push_str("\r\n");
    for row in rows {
        for index in 0..columns.len() {
            if index > 0 {
                text.push(',');
            }
            push_field(&mut text, row.get(index).and_then(|value| value.as_deref()));
        }
        text.push_str("\r\n");
    }
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(text.as_bytes());
    bytes
}

/// Rewrites only the table name in a `CREATE TABLE` statement, keeping the
/// original column definitions, constraints and options. Used by SQLite table
/// cloning (MySQL/PostgreSQL have native `LIKE` clones).
pub fn rewrite_create_table(ddl: &str, new_name: &str) -> Result<String, String> {
    let lower = ddl.to_ascii_lowercase();
    let Some(prefix) = lower.find("create table") else {
        return Err("This table has no CREATE TABLE statement to clone.".to_string());
    };
    let bytes = ddl.as_bytes();
    let mut i = prefix + "create table".len();
    let skip_space = |i: &mut usize| {
        while *i < bytes.len() && bytes[*i].is_ascii_whitespace() {
            *i += 1;
        }
    };
    skip_space(&mut i);
    if lower[i..].starts_with("if not exists") {
        i += "if not exists".len();
        skip_space(&mut i);
    }
    let start = i;
    let end = match bytes.get(i).copied() {
        Some(b'"') | Some(b'`') => {
            let quote = bytes[i];
            i += 1;
            loop {
                if i >= bytes.len() {
                    break;
                }
                if bytes[i] == quote {
                    if bytes.get(i + 1) == Some(&quote) {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            i
        }
        Some(b'[') => {
            i += 1;
            while i < bytes.len() && bytes[i] != b']' {
                i += 1;
            }
            if i < bytes.len() {
                i += 1;
            }
            i
        }
        _ => {
            while i < bytes.len()
                && !bytes[i].is_ascii_whitespace()
                && bytes[i] != b'('
            {
                i += 1;
            }
            i
        }
    };
    if end <= start {
        return Err("Could not find the table name in its CREATE TABLE statement.".to_string());
    }
    let mut out = String::with_capacity(ddl.len() + new_name.len());
    out.push_str(&ddl[..start]);
    out.push_str(&quote_ident(new_name));
    out.push_str(&ddl[end..]);
    Ok(out)
}

/// Quotes a value as a SQL string literal.
fn sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Keeps SQL keywords and numbers in a DEFAULT clause bare, and quotes
/// anything else so a plain word becomes a valid string default.
fn default_expr(text: &str) -> String {
    let trimmed = text.trim();
    let upper = trimmed.to_ascii_uppercase();
    let is_number = trimmed.parse::<f64>().is_ok();
    let is_keyword =
        matches!(upper.as_str(), "NULL" | "TRUE" | "FALSE") || upper.starts_with("CURRENT_");
    if is_number || is_keyword {
        trimmed.to_string()
    } else {
        sql_literal(trimmed)
    }
}

pub fn list_tables(conn: &Connection) -> Result<Vec<TableInfo>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT name, type FROM sqlite_master
             WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%'
             ORDER BY type, name COLLATE NOCASE",
        )
        .map_err(|e| e.to_string())?;
    let mut names: Vec<(String, String)> = Vec::new();
    {
        let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
        while let Some(row) = rows.next().map_err(|e| e.to_string())? {
            let name: String = row.get(0).map_err(|e| e.to_string())?;
            let kind: String = row.get(1).map_err(|e| e.to_string())?;
            names.push((name, kind));
        }
    }

    let mut out = Vec::new();
    for (name, kind) in names {
        let is_view = kind == "view";
        let rows = if is_view {
            None
        } else {
            let sql = format!("SELECT COUNT(*) FROM {}", quote_ident(&name));
            conn.query_row(&sql, [], |r| r.get::<_, i64>(0)).ok()
        };
        out.push(TableInfo {
            name,
            schema: None,
            is_view,
            rows,
        });
    }
    Ok(out)
}

pub fn table_structure(conn: &Connection, table: &str) -> Result<Vec<ColumnMeta>, String> {
    let mut stmt = conn
        .prepare("SELECT name, type, \"notnull\", dflt_value, pk FROM pragma_table_info(?1)")
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let mut rows = stmt.query([table]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        let notnull: i64 = row.get(2).unwrap_or(0);
        let pk: i64 = row.get(4).unwrap_or(0);
        out.push(ColumnMeta {
            name: row.get(0).unwrap_or_default(),
            decl_type: row
                .get::<_, Option<String>>(1)
                .unwrap_or_default()
                .unwrap_or_default(),
            not_null: notnull != 0,
            default_value: row.get(3).ok(),
            primary_key: pk != 0,
            options: Vec::new(),
        });
    }
    // Closed sets declared as `CHECK (column IN (...))` become dropdowns.
    if let Some(ddl) = table_ddl(conn, table) {
        let mut with_options = Vec::with_capacity(out.len());
        for column in out {
            let options = sqlite_check_options(&ddl, &column.name);
            with_options.push(ColumnMeta { options, ..column });
        }
        out = with_options;
    }
    Ok(out)
}

pub fn table_indexes(conn: &Connection, table: &str) -> Vec<IndexMeta> {
    let mut out = Vec::new();
    let mut stmt = match conn.prepare("SELECT name, \"unique\" FROM pragma_index_list(?1)") {
        Ok(s) => s,
        Err(_) => return out,
    };
    let mut rows = match stmt.query([table]) {
        Ok(r) => r,
        Err(_) => return out,
    };
    let mut indexes: Vec<(String, bool)> = Vec::new();
    while let Ok(Some(row)) = rows.next() {
        let name: String = row.get(0).unwrap_or_default();
        let unique: i64 = row.get(1).unwrap_or(0);
        indexes.push((name, unique != 0));
    }
    for (name, unique) in indexes {
        let mut columns = Vec::new();
        if let Ok(mut info) = conn.prepare("SELECT name FROM pragma_index_info(?1) ORDER BY seqno")
        {
            if let Ok(mut irows) = info.query([&name]) {
                while let Ok(Some(row)) = irows.next() {
                    if let Ok(Some(col)) = row.get::<_, Option<String>>(0) {
                        columns.push(col);
                    }
                }
            }
        }
        out.push(IndexMeta {
            name,
            unique,
            columns,
        });
    }
    out
}

pub fn table_ddl(conn: &Connection, table: &str) -> Option<String> {
    conn.query_row(
        "SELECT sql FROM sqlite_master WHERE name = ?1",
        [table],
        |r| r.get::<_, Option<String>>(0),
    )
    .ok()
    .flatten()
}

// ---------------------------------------------------------------------------
// Query execution
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct QueryOutcome {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    pub rows_affected: usize,
    pub elapsed_ms: u128,
    pub statements: usize,
    pub message: String,
    pub error: Option<String>,
    pub truncated: bool,
}

impl QueryOutcome {
    pub fn ok_message(&self) -> String {
        if !self.message.is_empty() {
            self.message.clone()
        } else if !self.columns.is_empty() {
            format!(
                "{} row{} returned",
                self.rows.len(),
                if self.rows.len() == 1 { "" } else { "s" }
            )
        } else {
            format!(
                "{} row{} affected",
                self.rows_affected,
                if self.rows_affected == 1 { "" } else { "s" }
            )
        }
    }
}

/// Default number of rows read into one result view. Zero means no limit.
/// TablePlus stores no default key (`LimitQueryRowsReturned` is absent from
/// `com.tinyapp.TablePlus`), so the first menu entry, "No limit" (tag 0), is
/// the closest recovered behaviour.
pub const DEFAULT_ROW_LIMIT: usize = 0;

/// Largest page limit accepted by the original's paging popover.
/// `sub_10071b5e4` rejects `limit >= 0x989681` (10,000,001).
pub const MAX_ROW_LIMIT: i64 = 10_000_000;

/// Initial rows requested when a table tab opens.
///
/// The paging code falls back to `0x12c` (300) when the stored limit is below
/// that value, so 300 is the effective page size on a fresh table view. See
/// `sub_10071b144` (0x10071b144), `sub_10071b370` (0x10071b370) and
/// `sub_10081f074` (0x10081f074).
pub const DEFAULT_TABLE_PAGE_LIMIT: usize = 300;

/// Smallest page step used by the previous/next page actions.
///
/// All three paging functions load the stored limit, materialise `0x12c`, and
/// choose 300 whenever the stored limit is lower (including "No limit", 0):
/// `csel x, 0x12c, limit, lt`. The step is therefore `max(limit, 300)`.
pub const MIN_PAGE_STEP: usize = 300;

/// Number of rows moved by one page action.
pub fn page_step(limit: usize) -> usize {
    limit.max(MIN_PAGE_STEP)
}

/// Parses the Limit/Offset text exactly like `sub_100532a80` (0x100532a80):
/// an optional sign followed only by decimal digits. Any other text, including
/// leading or trailing whitespace, becomes 0.
pub fn parse_paging_text(text: &str) -> i64 {
    text.parse::<i64>().unwrap_or(0)
}

/// Paging rules recovered from `sub_10071b5e4` (0x10071b5e4).
///
/// The original compares `limit` against `0x989681` (10,000,001), so the
/// largest accepted value is 10,000,000, checks the offset sign, then rejects
/// `offset >= total_rows` only when `offset != 0`. The total is read from the
/// controller's `+0x620` slot, so a positive offset against an empty result is
/// still an error.
pub fn validate_paging(limit: i64, offset: i64, total: i64) -> Result<(), &'static str> {
    if limit <= 0 {
        return Err("The limitation should be greater than zero");
    }
    if limit > MAX_ROW_LIMIT {
        return Err("The limitation is too large");
    }
    if offset < 0 {
        return Err("The offset should be positive");
    }
    if offset != 0 && offset >= total {
        return Err("The offset should be smaller than the total of rows");
    }
    Ok(())
}

/// Blob cells carry hex (`0x...`) so the grid and the hex viewer show the
/// actual bytes. Very large values are truncated to keep a page responsive.
fn blob_hex_value(bytes: &[u8]) -> String {
    const MAX: usize = 2048;
    let shown = &bytes[..bytes.len().min(MAX)];
    let mut out = String::with_capacity(shown.len() * 2 + 2);
    out.push_str("0x");
    for byte in shown {
        out.push_str(&format!("{byte:02x}"));
    }
    if bytes.len() > MAX {
        out.push_str(&format!("… ({} bytes)", bytes.len()));
    }
    out
}

fn cell_from_value(value: ValueRef<'_>) -> Option<String> {
    match value {
        ValueRef::Null => None,
        ValueRef::Integer(i) => Some(i.to_string()),
        ValueRef::Real(f) => {
            if f.fract() == 0.0 && f.abs() < 1e15 {
                Some(format!("{f:.0}"))
            } else {
                Some(format!("{f}"))
            }
        }
        ValueRef::Text(t) => Some(String::from_utf8_lossy(t).to_string()),
        ValueRef::Blob(b) => Some(blob_hex_value(b)),
    }
}

fn run_single(
    conn: &Connection,
    sql: &str,
    max_rows: usize,
    outcome: &mut QueryOutcome,
) -> Result<(), String> {
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    outcome.statements += 1;

    if stmt.column_count() == 0 {
        let affected = stmt.execute([]).map_err(|e| e.to_string())?;
        outcome.rows_affected += affected;
        return Ok(());
    }

    let names: Vec<String> = stmt.column_names().iter().map(|n| n.to_string()).collect();
    if outcome.columns.is_empty() {
        outcome.columns = names;
    } else if outcome.columns != names {
        // A later statement with a different shape replaces the previous grid.
        outcome.columns = names;
        outcome.rows.clear();
    }

    let mut rows = stmt.query([]).map_err(|e| e.to_string())?;
    while let Some(row) = rows.next().map_err(|e| e.to_string())? {
        if max_rows > 0 && outcome.rows.len() >= max_rows {
            outcome.truncated = true;
            break;
        }
        let mut values = Vec::with_capacity(outcome.columns.len());
        for i in 0..outcome.columns.len() {
            values.push(cell_from_value(row.get_ref(i).map_err(|e| e.to_string())?));
        }
        outcome.rows.push(values);
    }
    Ok(())
}

#[allow(dead_code)]
/// Keywords that make a statement a write. Used by read-only mode; quoted
/// identifiers and string literals are skipped, and `FOR UPDATE` is not a
/// write on its own.
const WRITE_KEYWORDS: &[&str] = &[
    "INSERT", "UPDATE", "DELETE", "REPLACE", "CREATE", "DROP", "ALTER", "TRUNCATE", "GRANT",
    "REVOKE", "COPY", "CALL", "DO", "EXEC", "EXECUTE", "VACUUM", "ANALYZE", "ATTACH", "DETACH",
    "MERGE", "REINDEX", "REFRESH", "COMMENT", "RENAME", "UPSERT",
];

/// True when running `sql` could modify the database. Multiple statements are
/// checked one by one; `EXPLAIN` without `ANALYZE` is always treated as a read.
pub fn statement_writes(sql: &str) -> bool {
    let statements = split_statements(sql);
    if statements.is_empty() {
        return false;
    }
    statements
        .iter()
        .any(|statement| single_statement_writes(statement))
}

fn single_statement_writes(sql: &str) -> bool {
    let words = sql_words(sql);
    if words.is_empty() {
        return false;
    }
    if words[0].eq_ignore_ascii_case("EXPLAIN")
        && !words.iter().any(|word| word.eq_ignore_ascii_case("ANALYZE"))
    {
        return false;
    }
    let mut previous = "";
    for word in &words {
        let upper = word.to_ascii_uppercase();
        if previous.eq_ignore_ascii_case("FOR") && upper == "UPDATE" {
            previous = word;
            continue;
        }
        if WRITE_KEYWORDS.contains(&upper.as_str()) {
            return true;
        }
        previous = word;
    }
    false
}

/// Words outside strings, quoted identifiers and comments.
fn sql_words(sql: &str) -> Vec<String> {
    let mut words = Vec::new();
    let chars: Vec<char> = sql.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '-' && chars.get(i + 1) == Some(&'-') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(chars.len());
            continue;
        }
        // Quotes are skipped entirely, including doubled escapes.
        if matches!(c, '\'' | '"' | '`' | '[') {
            let close = if c == '[' { ']' } else { c };
            i += 1;
            while i < chars.len() {
                if chars[i] == close {
                    if close != ']' && chars.get(i + 1) == Some(&close) {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if c.is_alphanumeric() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            words.push(chars[start..i].iter().collect());
            continue;
        }
        i += 1;
    }
    words
}


/// Splits SQL on semicolons that are outside strings and comments.
/// Runs a script against a bare SQLite connection. Used by the tests; the app
/// goes through `Db::run_sql` so read-only and transactions apply.
#[cfg(test)]
pub fn run_sql(conn: &Connection, sql: &str, max_rows: usize) -> QueryOutcome {
    let start = Instant::now();
    let mut outcome = QueryOutcome::default();
    let statements = split_statements(sql);
    if statements.is_empty() {
        outcome.error = Some("There is no query to run.".to_string());
        outcome.elapsed_ms = start.elapsed().as_millis();
        return outcome;
    }
    for statement in statements {
        if outcome.error.is_some() {
            break;
        }
        if let Err(err) = run_single(conn, &statement, max_rows, &mut outcome) {
            outcome.error = Some(err);
        }
    }
    outcome.elapsed_ms = start.elapsed().as_millis();
    if outcome.error.is_none() && outcome.truncated {
        outcome.message = format!("Showing the first {max_rows} rows.");
    }
    outcome
}

pub fn split_statements(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = sql.chars().collect();
    let mut i = 0;
    let mut state = Scan::Normal;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match state {
            Scan::Normal => {
                if c == '\'' {
                    state = Scan::Single;
                    current.push(c);
                } else if c == '"' {
                    state = Scan::Double;
                    current.push(c);
                } else if c == '-' && next == Some('-') {
                    state = Scan::LineComment;
                    current.push(c);
                } else if c == '/' && next == Some('*') {
                    state = Scan::BlockComment;
                    current.push(c);
                } else if c == ';' {
                    push_statement(&mut statements, &mut current);
                } else {
                    current.push(c);
                }
            }
            Scan::Single => {
                current.push(c);
                if c == '\'' {
                    if next == Some('\'') {
                        current.push('\'');
                        i += 1;
                    } else {
                        state = Scan::Normal;
                    }
                }
            }
            Scan::Double => {
                current.push(c);
                if c == '"' {
                    if next == Some('"') {
                        current.push('"');
                        i += 1;
                    } else {
                        state = Scan::Normal;
                    }
                }
            }
            Scan::LineComment => {
                current.push(c);
                if c == '\n' {
                    state = Scan::Normal;
                }
            }
            Scan::BlockComment => {
                current.push(c);
                if c == '*' && next == Some('/') {
                    current.push('/');
                    i += 1;
                    state = Scan::Normal;
                }
            }
        }
        i += 1;
    }
    push_statement(&mut statements, &mut current);
    statements
}

enum Scan {
    Normal,
    Single,
    Double,
    LineComment,
    BlockComment,
}

fn push_statement(statements: &mut Vec<String>, current: &mut String) {
    let trimmed = current.trim().trim_end_matches(';').trim().to_string();
    current.clear();
    if trimmed.is_empty() {
        return;
    }
    let without_comments = trimmed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    if !without_comments.is_empty() {
        statements.push(trimmed);
    }
}

// ---------------------------------------------------------------------------
// Table browsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct TablePage {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
    /// Values of `key_column` that identify each row for cell editing.
    pub rowids: Vec<Option<String>>,
    /// Column used to address a row (rowid, primary key or ctid). None disables editing.
    pub key_column: Option<String>,
    pub total: i64,
    pub elapsed_ms: u128,
    pub error: Option<String>,
    pub editable: bool,
}

// ---------------------------------------------------------------------------
// Table filters
// ---------------------------------------------------------------------------

/// Comparison used by a column filter. Matching is case-insensitive on every
/// backend, like the recovered Quick Filter operators (`sub_1006f95a0`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum FilterOp {
    Contains,
    NotContains,
    StartsWith,
    EndsWith,
    Equals,
    NotEquals,
    IsEmpty,
    IsNotEmpty,
}

impl FilterOp {
    /// Operators offered in the picker, most useful first.
    pub const ALL: [FilterOp; 8] = [
        FilterOp::Contains,
        FilterOp::NotContains,
        FilterOp::StartsWith,
        FilterOp::EndsWith,
        FilterOp::Equals,
        FilterOp::NotEquals,
        FilterOp::IsEmpty,
        FilterOp::IsNotEmpty,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FilterOp::Contains => "contains",
            FilterOp::NotContains => "does not contain",
            FilterOp::StartsWith => "starts with",
            FilterOp::EndsWith => "ends with",
            FilterOp::Equals => "equals",
            FilterOp::NotEquals => "not equals",
            FilterOp::IsEmpty => "is empty",
            FilterOp::IsNotEmpty => "is not empty",
        }
    }

    /// `is empty` and `is not empty` do not read the value field.
    pub fn needs_value(self) -> bool {
        !matches!(self, FilterOp::IsEmpty | FilterOp::IsNotEmpty)
    }
}

/// One column-scoped filter: `column` compared with `value` using `op`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColumnFilter {
    pub column: String,
    pub op: FilterOp,
    pub value: String,
}

impl ColumnFilter {
    pub fn new(column: impl Into<String>) -> Self {
        ColumnFilter {
            column: column.into(),
            op: FilterOp::Contains,
            value: String::new(),
        }
    }

    /// A row in the filter bar only constrains the query once it has a column
    /// and either a value or an operator that ignores the value.
    pub fn is_active(&self) -> bool {
        !self.column.is_empty() && (!self.op.needs_value() || !self.value.trim().is_empty())
    }
}

/// What a table tab filters on: the global Quick Filter across every column
/// plus the column-scoped filters (the original saves those as
/// `DefaultAvancedFilter*` keys in `ViewSetting`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TableFilter {
    pub quick: String,
    pub columns: Vec<ColumnFilter>,
    /// Combine the clauses with OR instead of AND.
    pub any: bool,
}

impl TableFilter {
    pub fn is_empty(&self) -> bool {
        self.quick.trim().is_empty() && self.active_columns().count() == 0
    }

    pub fn active_columns(&self) -> impl Iterator<Item = &ColumnFilter> {
        self.columns.iter().filter(|f| f.is_active())
    }

    /// Number of clauses the filter actually applies (for the toolbar badge).
    pub fn active_count(&self) -> usize {
        usize::from(!self.quick.trim().is_empty()) + self.active_columns().count()
    }
}

/// One ORDER BY term chosen in a grid header. Columns are validated against
/// the table structure before they reach SQL.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SortSpec {
    pub column: String,
    pub descending: bool,
}

/// SQL differences the filter builder has to respect.
#[derive(Clone, Copy)]
enum Dialect {
    Sqlite,
    MySql,
    Postgres,
}

impl Dialect {
    fn quote(self, name: &str) -> String {
        match self {
            Dialect::MySql => format!("`{}`", name.replace('`', "``")),
            _ => quote_ident(name),
        }
    }

    /// Type a column is cast to before `LOWER`.
    fn cast_text(self) -> &'static str {
        match self {
            Dialect::MySql => "CHAR",
            _ => "TEXT",
        }
    }

    /// Inline literal for a value inside a self-contained SQL script.
    fn literal(self, value: &str) -> String {
        match self {
            Dialect::MySql => mysql_literal(value),
            _ => sql_literal(value),
        }
    }
}

/// Builds the `WHERE` body for a filter. SQLite and MySQL get `?` placeholders
/// and collect the bound values in `params`; the PostgreSQL path only runs
/// literal SQL, so there the values are escaped into the clauses.
fn build_where(
    filter: &TableFilter,
    columns: &[String],
    dialect: Dialect,
) -> (String, Vec<String>) {
    build_where_with(filter, columns, dialect, false)
}

/// Same clauses as [`build_where`], but every value is an inline SQL literal.
/// Used by SQL dumps, which are a self-contained script with no parameters.
fn build_where_literals(filter: &TableFilter, columns: &[String], dialect: Dialect) -> String {
    build_where_with(filter, columns, dialect, true).0
}

fn build_where_with(
    filter: &TableFilter,
    columns: &[String],
    dialect: Dialect,
    literals: bool,
) -> (String, Vec<String>) {
    fn operand(
        dialect: Dialect,
        value: &str,
        params: &mut Vec<String>,
        literals: bool,
    ) -> String {
        match dialect {
            Dialect::Postgres => pg_literal(value),
            _ if literals => dialect.literal(value),
            _ => {
                params.push(value.to_string());
                "?".to_string()
            }
        }
    }
    fn contains_pattern(value: &str) -> String {
        format!("%{}%", like_escape(&value.to_lowercase()))
    }

    let mut params: Vec<String> = Vec::new();
    let mut clauses: Vec<String> = Vec::new();

    let quick = filter.quick.trim();
    if !quick.is_empty() {
        let pattern = contains_pattern(quick);
        let parts: Vec<String> = columns
            .iter()
            .map(|column| {
                let expr = format!(
                    "LOWER(CAST({} AS {}))",
                    dialect.quote(column),
                    dialect.cast_text()
                );
                let place = operand(dialect, &pattern, &mut params, literals);
                match dialect {
                    // SQLite only honours the LIKE wildcard escaping when the
                    // escape character is spelled out.
                    Dialect::Sqlite => format!("{expr} LIKE {place} ESCAPE '\\'"),
                    _ => format!("{expr} LIKE {place}"),
                }
            })
            .collect();
        match parts.len() {
            0 => {}
            1 => clauses.push(parts[0].clone()),
            _ => clauses.push(format!("({})", parts.join(" OR "))),
        }
    }

    for column_filter in filter.active_columns() {
        if !columns.iter().any(|c| c == &column_filter.column) {
            // The column disappeared (schema change): skip the row instead of
            // generating invalid SQL.
            continue;
        }
        let column = dialect.quote(&column_filter.column);
        let expr = format!("LOWER(CAST({column} AS {}))", dialect.cast_text());
        let value = column_filter.value.trim().to_lowercase();
        match column_filter.op {
            FilterOp::Contains => clauses.push(format!(
                "{expr} LIKE {}",
                operand(dialect, &contains_pattern(&value), &mut params, literals)
            )),
            FilterOp::NotContains => {
                // NULL cells do not contain the text, so negative filters keep
                // them visible in a data browser.
                let place = operand(dialect, &contains_pattern(&value), &mut params, literals);
                clauses.push(format!("({column} IS NULL OR {expr} NOT LIKE {place})"));
            }
            FilterOp::StartsWith => {
                let pattern = format!("{}%", like_escape(&value));
                clauses.push(format!(
                    "{expr} LIKE {}",
                    operand(dialect, &pattern, &mut params, literals)
                ));
            }
            FilterOp::EndsWith => {
                let pattern = format!("%{}", like_escape(&value));
                clauses.push(format!(
                    "{expr} LIKE {}",
                    operand(dialect, &pattern, &mut params, literals)
                ));
            }
            FilterOp::Equals => clauses.push(format!(
                "{expr} = {}",
                operand(dialect, &value, &mut params, literals)
            )),
            FilterOp::NotEquals => {
                let place = operand(dialect, &value, &mut params, literals);
                clauses.push(format!("({column} IS NULL OR {expr} <> {place})"));
            }
            FilterOp::IsEmpty => clauses.push(format!(
                "({column} IS NULL OR CAST({column} AS {}) = '')",
                dialect.cast_text()
            )),
            FilterOp::IsNotEmpty => clauses.push(format!(
                "({column} IS NOT NULL AND CAST({column} AS {}) <> '')",
                dialect.cast_text()
            )),
        }
    }

    let where_sql = match clauses.len() {
        0 => String::new(),
        1 => format!("WHERE {}", clauses[0]),
        _ if filter.any => format!("WHERE ({})", clauses.join(" OR ")),
        _ => format!("WHERE {}", clauses.join(" AND ")),
    };
    (where_sql, params)
}

/// Builds an `ORDER BY` clause from validated sort terms. Unknown columns are
/// ignored so a stale sort cannot produce invalid SQL.
fn build_order_by(sort: &[SortSpec], columns: &[String], dialect: Dialect) -> String {
    let parts: Vec<String> = sort
        .iter()
        .filter(|spec| columns.iter().any(|column| column == &spec.column))
        .map(|spec| {
            format!(
                "{} {}",
                dialect.quote(&spec.column),
                if spec.descending { "DESC" } else { "ASC" }
            )
        })
        .collect();
    if parts.is_empty() {
        String::new()
    } else {
        format!("ORDER BY {}", parts.join(", "))
    }
}

/// Sorts an in-memory result set (query tabs) with the same column order the
/// user clicked in the header. NULLs stay last in both directions; numeric
/// cells compare as numbers, everything else as case-insensitive text.
pub fn sort_rows(rows: &mut [Vec<Option<String>>], sort: &[SortSpec], columns: &[String]) {
    let indices: Vec<(usize, bool)> = sort
        .iter()
        .filter_map(|spec| {
            columns
                .iter()
                .position(|column| column == &spec.column)
                .map(|index| (index, spec.descending))
        })
        .collect();
    if indices.is_empty() {
        return;
    }
    rows.sort_by(|left, right| {
        for &(index, descending) in &indices {
            let left = left.get(index).and_then(|value| value.as_deref());
            let right = right.get(index).and_then(|value| value.as_deref());
            let ordering = match (left, right) {
                (None, None) => Ordering::Equal,
                (None, _) => Ordering::Greater,
                (_, None) => Ordering::Less,
                (Some(left), Some(right)) => compare_sort_values(left, right, descending),
            };
            if ordering != Ordering::Equal {
                return ordering;
            }
        }
        Ordering::Equal
    });
}

fn compare_sort_values(left: &str, right: &str, descending: bool) -> Ordering {
    let ordering = match (left.parse::<f64>(), right.parse::<f64>()) {
        (Ok(left), Ok(right)) => left.partial_cmp(&right).unwrap_or(Ordering::Equal),
        _ => left.to_lowercase().cmp(&right.to_lowercase()),
    };
    if descending {
        ordering.reverse()
    } else {
        ordering
    }
}

/// MySQL string literal with backslash escaping. `pg_literal`/`sql_literal`
/// are not enough because MySQL treats `\` as an escape character by default.
fn mysql_literal(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for c in value.chars() {
        match c {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            '\0' => out.push_str("\\0"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\x1a' => out.push_str("\\Z"),
            other => out.push(other),
        }
    }
    out.push('\'');
    out
}

fn like_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    for c in value.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

pub fn fetch_table(
    conn: &Connection,
    table: &str,
    limit: usize,
    offset: usize,
    filter: &TableFilter,
    sort: &[SortSpec],
) -> TablePage {
    let start = Instant::now();
    let mut out = TablePage::default();
    let columns = match table_structure(conn, table) {
        Ok(c) => c,
        Err(e) => {
            out.error = Some(e);
            return out;
        }
    };
    if columns.is_empty() {
        out.error = Some("This table has no columns.".to_string());
        out.elapsed_ms = start.elapsed().as_millis();
        return out;
    }
    let names: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
    let (where_sql, params) = build_where(filter, &names, Dialect::Sqlite);

    let count_sql = format!("SELECT COUNT(*) FROM {} {}", quote_ident(table), where_sql);
    let count = conn.query_row(&count_sql, rusqlite::params_from_iter(params.iter()), |r| {
        r.get::<_, i64>(0)
    });
    out.total = count.unwrap_or(0);

    // SQLite needs LIMIT -1 to keep OFFSET valid when no limit is set.
    let limit_sql = if limit == 0 {
        "-1".to_string()
    } else {
        limit.to_string()
    };
    let sort_order = build_order_by(sort, &names, Dialect::Sqlite);
    let base_order = if sort_order.is_empty() {
        "ORDER BY rowid".to_string()
    } else {
        format!("{sort_order}, rowid")
    };
    // Views and WITHOUT ROWID tables have no rowid, so the fallback cannot
    // use it as a tiebreaker.
    let fallback_order = sort_order;
    let base = format!(
        "SELECT rowid, * FROM {} {} {} LIMIT {} OFFSET {}",
        quote_ident(table),
        where_sql,
        base_order,
        limit_sql,
        offset
    );
    let fallback = format!(
        "SELECT * FROM {} {} {} LIMIT {} OFFSET {}",
        quote_ident(table),
        where_sql,
        fallback_order,
        limit_sql,
        offset
    );

    // The parameters are bound separately because rusqlite needs the same
    // placeholder shape in both statements.
    let mut run_with_filter = |sql: &str, with_rowid: bool| -> Result<(), String> {
        let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
        let start_idx = if with_rowid { 1 } else { 0 };
        let result_cols: Vec<String> = stmt
            .column_names()
            .iter()
            .skip(start_idx)
            .map(|n| n.to_string())
            .collect();
        out.columns = result_cols;
        let mut collected: Vec<Vec<Option<String>>> = Vec::new();
        let mut rowids: Vec<Option<String>> = Vec::new();
        {
            let mut rows = stmt
                .query(rusqlite::params_from_iter(params.iter()))
                .map_err(|e| e.to_string())?;
            while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                let mut rowid = None;
                if with_rowid {
                    rowid = row.get::<_, Option<i64>>(0).ok().flatten();
                }
                let mut values = Vec::new();
                for i in 0..out.columns.len() {
                    let index = if with_rowid { i + 1 } else { i };
                    values.push(cell_from_value(
                        row.get_ref(index).map_err(|e| e.to_string())?,
                    ));
                }
                collected.push(values);
                rowids.push(rowid.map(|v| v.to_string()));
            }
        }
        out.rows = collected;
        out.rowids = rowids;
        out.editable = with_rowid;
        out.key_column = if with_rowid {
            Some("rowid".to_string())
        } else {
            None
        };
        Ok(())
    };

    let result = match run_with_filter(&base, true) {
        Ok(()) => Ok(()),
        Err(_) => run_with_filter(&fallback, false),
    };
    if let Err(e) = result {
        out.error = Some(e);
        out.rows.clear();
        out.rowids.clear();
    }
    out.elapsed_ms = start.elapsed().as_millis();
    out
}

#[allow(dead_code)]
pub fn update_cell(
    conn: &Connection,
    table: &str,
    column: &str,
    rowid: i64,
    value: &str,
) -> Result<usize, String> {
    let sql = format!(
        "UPDATE {} SET {} = ?1 WHERE rowid = ?2",
        quote_ident(table),
        quote_ident(column)
    );
    conn.execute(&sql, rusqlite::params![value, rowid])
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Sample database
// ---------------------------------------------------------------------------

pub fn ensure_sample_database() -> Result<PathBuf, String> {
    let dir = app_data_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("Could not create the data folder: {e}"))?;
    let path = sample_db_path();
    let conn = Connection::open(&path).map_err(|e| format!("Could not create the sample: {e}"))?;
    let existing: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'customers'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if existing == 0 {
        seed_sample(&conn)?;
    }
    Ok(path)
}

fn seed_sample(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        r#"
        CREATE TABLE customers (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            email TEXT NOT NULL,
            country TEXT NOT NULL,
            joined_on TEXT NOT NULL
        );
        CREATE TABLE products (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            category TEXT NOT NULL,
            price REAL NOT NULL,
            stock INTEGER NOT NULL,
            active INTEGER NOT NULL DEFAULT 1
        );
        CREATE TABLE orders (
            id INTEGER PRIMARY KEY,
            customer_id INTEGER NOT NULL REFERENCES customers(id),
            placed_on TEXT NOT NULL,
            status TEXT NOT NULL,
            total REAL NOT NULL
        );
        CREATE TABLE order_items (
            id INTEGER PRIMARY KEY,
            order_id INTEGER NOT NULL REFERENCES orders(id),
            product_id INTEGER NOT NULL REFERENCES products(id),
            quantity INTEGER NOT NULL,
            unit_price REAL NOT NULL
        );
        CREATE TABLE employees (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            role TEXT NOT NULL,
            city TEXT NOT NULL,
            hired_on TEXT NOT NULL,
            salary INTEGER NOT NULL
        );
        CREATE INDEX idx_orders_customer ON orders(customer_id);
        CREATE INDEX idx_orders_status ON orders(status);
        CREATE VIEW active_products AS
            SELECT id, name, category, price, stock
            FROM products WHERE active = 1;
        "#,
    )
    .map_err(|e| e.to_string())?;

    let customers = [
        (
            "Avery Chen",
            "avery.chen@example.com",
            "Singapore",
            "2022-03-14",
        ),
        (
            "Mila Novak",
            "mila.novak@example.com",
            "Czechia",
            "2022-05-02",
        ),
        (
            "Noah Bennett",
            "noah.bennett@example.com",
            "United Kingdom",
            "2022-06-21",
        ),
        (
            "Sofia Rossi",
            "sofia.rossi@example.com",
            "Italy",
            "2022-08-09",
        ),
        (
            "Liam Okafor",
            "liam.okafor@example.com",
            "Nigeria",
            "2022-09-30",
        ),
        ("Hana Sato", "hana.sato@example.com", "Japan", "2022-11-11"),
        (
            "Mateo Alvarez",
            "mateo.alvarez@example.com",
            "Spain",
            "2023-01-05",
        ),
        (
            "Ingrid Larsen",
            "ingrid.larsen@example.com",
            "Norway",
            "2023-02-18",
        ),
        (
            "Priya Raman",
            "priya.raman@example.com",
            "India",
            "2023-03-27",
        ),
        (
            "Elias Weber",
            "elias.weber@example.com",
            "Germany",
            "2023-04-12",
        ),
        (
            "Amara Diallo",
            "amara.diallo@example.com",
            "Senegal",
            "2023-05-23",
        ),
        (
            "Owen Murphy",
            "owen.murphy@example.com",
            "Ireland",
            "2023-07-01",
        ),
        (
            "Zara Haddad",
            "zara.haddad@example.com",
            "Lebanon",
            "2023-08-16",
        ),
        (
            "Felix Kim",
            "felix.kim@example.com",
            "South Korea",
            "2023-09-08",
        ),
        (
            "Nora Lindqvist",
            "nora.lindqvist@example.com",
            "Sweden",
            "2023-10-19",
        ),
        (
            "Diego Costa",
            "diego.costa@example.com",
            "Brazil",
            "2023-11-29",
        ),
        (
            "Yara Mansour",
            "yara.mansour@example.com",
            "Egypt",
            "2024-01-10",
        ),
        (
            "Theo Martin",
            "theo.martin@example.com",
            "France",
            "2024-02-22",
        ),
        (
            "Ivy Thompson",
            "ivy.thompson@example.com",
            "Canada",
            "2024-04-03",
        ),
        (
            "Ravi Patel",
            "ravi.patel@example.com",
            "India",
            "2024-05-15",
        ),
        (
            "Clara Hoffmann",
            "clara.hoffmann@example.com",
            "Austria",
            "2024-06-26",
        ),
        (
            "Jonas Silva",
            "jonas.silva@example.com",
            "Portugal",
            "2024-08-07",
        ),
        ("Mei Lin", "mei.lin@example.com", "Taiwan", "2024-09-18"),
        (
            "Ade Okoye",
            "ade.okoye@example.com",
            "Nigeria",
            "2024-10-30",
        ),
    ];
    for (i, (name, email, country, joined)) in customers.iter().enumerate() {
        conn.execute(
            "INSERT INTO customers (id, name, email, country, joined_on) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![i as i64 + 1, name, email, country, joined],
        )
        .map_err(|e| e.to_string())?;
    }

    let products = [
        ("Harbor Mug", "Kitchen", 18.0, 120, 1),
        ("Field Notebook", "Stationery", 12.5, 240, 1),
        ("Cedar Desk Tray", "Office", 34.0, 48, 1),
        ("Linen Apron", "Kitchen", 42.0, 36, 1),
        ("Copper Pen", "Stationery", 24.0, 90, 1),
        ("Canvas Tote", "Bags", 28.0, 150, 1),
        ("Glass Carafe", "Kitchen", 39.0, 22, 1),
        ("Wool Throw", "Home", 76.0, 18, 1),
        ("Map Print", "Home", 22.0, 64, 1),
        ("Brass Ruler", "Office", 16.0, 85, 1),
        ("Stone Coaster", "Home", 9.5, 300, 1),
        ("Travel Pouch", "Bags", 31.0, 54, 1),
        ("Ceramic Bowl", "Kitchen", 26.0, 40, 0),
        ("Sketch Pad", "Stationery", 14.0, 110, 1),
        ("Oak Bookend", "Office", 45.0, 12, 0),
        ("Pocket Mirror", "Accessories", 19.0, 75, 1),
        ("Tea Tin", "Kitchen", 15.5, 130, 1),
        ("Leather Strap", "Accessories", 29.0, 44, 1),
    ];
    for (i, (name, category, price, stock, active)) in products.iter().enumerate() {
        conn.execute(
            "INSERT INTO products (id, name, category, price, stock, active) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![i as i64 + 1, name, category, price, stock, active],
        )
        .map_err(|e| e.to_string())?;
    }

    let statuses = ["paid", "shipped", "delivered", "refunded", "pending"];
    let mut order_id = 1;
    for i in 0..48i64 {
        let customer_id = (i % customers.len() as i64) + 1;
        let month = (i % 12) + 1;
        let day = (i % 26) + 1;
        let placed = format!("2024-{month:02}-{day:02}");
        let status = statuses[(i as usize) % statuses.len()];
        let total = 24.0 + ((i * 37) % 220) as f64 + (i % 4) as f64 * 0.5;
        conn.execute(
            "INSERT INTO orders (id, customer_id, placed_on, status, total) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![order_id, customer_id, placed, status, total],
        )
        .map_err(|e| e.to_string())?;

        let item_count = (i % 3) + 1;
        for j in 0..item_count {
            let product_id = ((i + j) % products.len() as i64) + 1;
            let quantity = ((i + j) % 4) + 1;
            let unit = products[(product_id - 1) as usize].2;
            conn.execute(
                "INSERT INTO order_items (order_id, product_id, quantity, unit_price) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![order_id, product_id, quantity, unit],
            )
            .map_err(|e| e.to_string())?;
        }
        order_id += 1;
    }

    let employees = [
        (
            "Rosa Delgado",
            "Store manager",
            "Lisbon",
            "2019-04-08",
            62000,
        ),
        ("Kenji Mori", "Buyer", "Osaka", "2020-01-27", 54000),
        (
            "Alina Petrova",
            "Support lead",
            "Tallinn",
            "2020-06-15",
            51000,
        ),
        ("Marcus Reed", "Warehouse", "Chicago", "2021-02-01", 44000),
        (
            "Fatima Zahra",
            "Designer",
            "Casablanca",
            "2021-08-19",
            49000,
        ),
        ("Tomas Berg", "Engineer", "Stockholm", "2022-03-07", 68000),
        ("Lena Fischer", "Accountant", "Berlin", "2022-09-12", 57000),
        ("Sam Carter", "Support", "Austin", "2023-01-23", 42000),
        ("Nia Mwangi", "Buyer", "Nairobi", "2023-05-04", 46000),
        ("Hugo Laurent", "Warehouse", "Lyon", "2023-10-16", 41000),
        ("Sara Ahmadi", "Engineer", "Tehran", "2024-02-05", 63000),
        ("Ben Whitaker", "Support", "Manchester", "2024-07-22", 43000),
    ];
    for (i, (name, role, city, hired, salary)) in employees.iter().enumerate() {
        conn.execute(
            "INSERT INTO employees (id, name, role, city, hired_on, salary) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![i as i64 + 1, name, role, city, hired, salary],
        )
        .map_err(|e| e.to_string())?;
    }

    Ok(())
}

pub fn default_query() -> String {
    "-- Dataloom · SQLite workspace
-- Press Command+Enter to run the current query.

SELECT c.name,
       c.country,
       COUNT(o.id)              AS orders,
       ROUND(SUM(o.total), 2)   AS lifetime_value
FROM customers c
LEFT JOIN orders o ON o.customer_id = c.id
GROUP BY c.id
ORDER BY lifetime_value DESC
LIMIT 12;"
        .to_string()
}

pub fn default_query_inspect() -> String {
    "-- Dataloom · SQLite workspace
-- Press Command+Enter to run the current query.

SELECT name, type
FROM sqlite_master
WHERE type IN ('table', 'view')
  AND name NOT LIKE 'sqlite_%'
ORDER BY type, name;"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mysql_saved(ssl_mode: &str) -> SavedConnection {
        SavedConnection {
            id: 1,
            name: "TLS".to_string(),
            path: String::new(),
            driver: "MySQL".to_string(),
            host: "db.example.com".to_string(),
            port: 3306,
            user: "app".to_string(),
            password: String::new(),
            database: "shop".to_string(),
            read_only: false,
            ssl_mode: ssl_mode.to_string(),
            ssl_root_cert: String::new(),
            ssh_host: String::new(),
            ssh_port: 0,
            ssh_user: String::new(),
            ssh_key_path: String::new(),
            group_id: None,
        }
    }

    #[test]
    fn write_classification_handles_comments_strings_and_for_update() {
        assert!(!statement_writes("SELECT * FROM t"));
        assert!(!statement_writes("  -- UPDATE t\nSELECT 1"));
        assert!(!statement_writes("SELECT 'update users set x=1'"));
        assert!(!statement_writes("SELECT * FROM t FOR UPDATE"));
        assert!(!statement_writes("EXPLAIN UPDATE t SET x = 1"));
        assert!(statement_writes("UPDATE t SET x = 1"));
        assert!(statement_writes("WITH x AS (SELECT 1) DELETE FROM t"));
        assert!(statement_writes("SELECT 1; DROP TABLE t"));
        assert!(statement_writes("/* note */ INSERT INTO t VALUES (1)"));
        assert!(statement_writes("CREATE TABLE t (x INTEGER)"));
        assert!(statement_writes("EXPLAIN ANALYZE UPDATE t SET x = 1"));
    }

    #[test]
    fn read_only_blocks_writes_until_toggled() {
        let dir = temp_dir("read-only");
        let path = dir.join("ro.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT);
                 INSERT INTO notes (body) VALUES ('hello');",
            )
            .unwrap();
        }
        let mut saved = saved_sqlite("Read only", &path);
        saved.read_only = true;
        let db = Db::open(&saved).unwrap();
        assert!(db.is_read_only());

        let read = db.run_sql("SELECT count(*) FROM notes", 0);
        assert_eq!(read.error, None);
        let write = db.run_sql("UPDATE notes SET body = 'x'", 0);
        assert!(write.error.unwrap_or_default().contains("Read-only"));
        let field = db
            .update_field("notes", "body", "rowid", "1", Some("x"))
            .unwrap_err();
        assert!(field.contains("Read-only"));

        db.set_read_only(false);
        assert!(!db.run_sql("BEGIN", 0).error.is_some());
        db.insert_row("notes", &[("body".to_string(), "second".to_string())])
            .unwrap();
        assert_eq!(db.run_sql("ROLLBACK", 0).error, None);
        let count = db.run_sql("SELECT count(*) FROM notes", 0);
        assert_eq!(count.rows[0][0].as_deref(), Some("1"));

        db.run_sql("BEGIN", 0);
        db.insert_row("notes", &[("body".to_string(), "third".to_string())])
            .unwrap();
        db.run_sql("COMMIT", 0);
        let count = db.run_sql("SELECT count(*) FROM notes", 0);
        assert_eq!(count.rows[0][0].as_deref(), Some("2"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn field_editors_are_chosen_from_declared_types() {
        assert_eq!(FieldEditor::from_decl("boolean", &[]), FieldEditor::Bool);
        assert_eq!(FieldEditor::from_decl("tinyint(1)", &[]), FieldEditor::Bool);
        assert_eq!(FieldEditor::from_decl("date", &[]), FieldEditor::Date);
        assert_eq!(
            FieldEditor::from_decl("timestamp without time zone", &[]),
            FieldEditor::DateTime
        );
        assert_eq!(FieldEditor::from_decl("jsonb", &[]), FieldEditor::Json);
        assert_eq!(FieldEditor::from_decl("bytea", &[]), FieldEditor::Blob);
        assert_eq!(FieldEditor::from_decl("text", &[]), FieldEditor::Text);
        assert_eq!(
            FieldEditor::from_decl("text", &["a".to_string()]),
            FieldEditor::Enum(vec!["a".to_string()])
        );
    }

    #[test]
    fn enum_types_and_check_constraints_become_options() {
        assert_eq!(
            parse_enum_type("enum('small','medium','large')"),
            vec!["small", "medium", "large"]
        );
        assert_eq!(
            parse_enum_type("ENUM('it''s','b')"),
            vec!["it's", "b"]
        );
        assert!(parse_enum_type("varchar(20)").is_empty());

        let ddl = "CREATE TABLE t (id INTEGER, status TEXT CHECK (status IN ('open','closed')))";
        assert_eq!(
            sqlite_check_options(ddl, "status"),
            vec!["open", "closed"]
        );
        assert!(sqlite_check_options(ddl, "other").is_empty());
    }

    #[test]
    fn mysql_ssl_modes_map_to_verification_flags() {
        assert!(mysql_ssl_opts(&mysql_saved("disable")).unwrap().is_none());

        let require = mysql_ssl_opts(&mysql_saved("require")).unwrap().unwrap();
        assert!(require.skip_domain_validation());
        assert!(require.accept_invalid_certs());

        let verify_ca = mysql_ssl_opts(&mysql_saved("verify-ca")).unwrap().unwrap();
        assert!(verify_ca.skip_domain_validation());
        assert!(!verify_ca.accept_invalid_certs());

        let missing = mysql_ssl_opts(&SavedConnection {
            ssl_root_cert: "/definitely/not/here.pem".to_string(),
            ..mysql_saved("verify-full")
        });
        assert!(missing.is_err());
    }

    #[test]
    #[ignore = "writes to the real macOS Keychain"]
    fn keychain_roundtrip() {
        let id = u64::MAX - 7;
        if let Err(error) = store_password(id, "dataloom-test-password") {
            eprintln!("Keychain unavailable, skipping: {error}");
            return;
        }
        store_password(id, "dataloom-test-password-2").unwrap();
        let loaded = load_password(id);
        delete_password(id);
        assert_eq!(loaded.as_deref(), Some("dataloom-test-password-2"));
        assert!(load_password(id).is_none(), "the test secret was not deleted");
    }

    #[test]
    fn postgres_tls_connector_builds_and_rejects_missing_ca() {
        assert!(postgres_tls_connector(&mysql_saved("require"), SslMode::Require).is_ok());
        let missing = SavedConnection {
            ssl_root_cert: "/definitely/not/here.pem".to_string(),
            ..mysql_saved("verify-full")
        };
        assert!(postgres_tls_connector(&missing, SslMode::VerifyFull).is_err());
    }

    #[test]
    fn legacy_connection_json_gets_tls_and_ssh_defaults() {
        let saved: SavedConnection =
            serde_json::from_str(r#"{"name":"Legacy","driver":"MySQL"}"#).unwrap();
        assert_eq!(saved.ssl_mode, "disable");
        assert!(saved.ssl_root_cert.is_empty());
        assert!(saved.ssh_host.is_empty());
        assert_eq!(saved.ssh_port, 0);
    }

    #[test]
    fn sample_connection_is_seeded_only_on_first_launch() {
        let empty = Store::default();
        // Very first launch: seed the sample so the app is not a blank slate.
        assert!(should_seed_sample(&empty, false));
        // Deleting every connection leaves an empty store on disk; it stays empty.
        assert!(!should_seed_sample(&empty, true));

        let mut store = Store::default();
        store.connections.push(SavedConnection {
            id: 1,
            name: "Local".to_string(),
            path: String::new(),
            driver: default_driver(),
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
        });
        assert!(!should_seed_sample(&store, false));
    }

    #[test]
    fn splits_statements_without_breaking_quotes_or_comments() {
        let sql = "SELECT ';' AS a; -- a comment ;\nSELECT 2;\n/* block ; */ SELECT 3";
        let parts = split_statements(sql);
        assert_eq!(parts.len(), 3);
        assert!(parts[0].contains("';'"));
        assert_eq!(parts[2], "/* block ; */ SELECT 3");
    }

    #[test]
    fn runs_queries_browses_tables_and_updates_cells() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT, city TEXT);
             INSERT INTO people (name, city) VALUES ('Avery', 'Oslo');
             INSERT INTO people (name, city) VALUES ('Mila', 'Paris');",
        )
        .unwrap();

        let out = run_sql(
            &conn,
            "SELECT name FROM people ORDER BY id;",
            DEFAULT_ROW_LIMIT,
        );
        assert_eq!(out.error, None);
        assert_eq!(out.rows.len(), 2);
        assert_eq!(out.rows[0][0].as_deref(), Some("Avery"));

        let page = fetch_table(&conn, "people", 100, 0, &TableFilter::default(), &[]);
        assert_eq!(page.error, None);
        assert_eq!(page.total, 2);
        assert!(page.editable);
        assert_eq!(page.rowids[0], Some("1".to_string()));
        assert_eq!(page.key_column.as_deref(), Some("rowid"));

        update_cell(&conn, "people", "name", 1, "Zoe").unwrap();
        let name: String = conn
            .query_row("SELECT name FROM people WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "Zoe");

        let quick = TableFilter {
            quick: "Zo".to_string(),
            ..TableFilter::default()
        };
        let filtered = fetch_table(&conn, "people", 100, 0, &quick, &[]);
        assert_eq!(filtered.rows.len(), 1);
        // `sub_1006f95a0` lowercases both sides for the default
        // "Contains - Case insensitive" operator.
        let quick_upper = TableFilter {
            quick: "ZO".to_string(),
            ..TableFilter::default()
        };
        let filtered_upper = fetch_table(&conn, "people", 100, 0, &quick_upper, &[]);
        assert_eq!(filtered_upper.rows.len(), 1);

        let structure = table_structure(&conn, "people").unwrap();
        assert_eq!(structure.len(), 3);
        assert!(structure[0].primary_key);

        conn.execute_batch("CREATE VIEW people_view AS SELECT name, city FROM people;")
            .unwrap();
        let view_page = fetch_table(&conn, "people_view", 100, 0, &TableFilter::default(), &[]);
        assert_eq!(view_page.error, None);
        assert_eq!(view_page.rows.len(), 2);
        assert!(!view_page.editable);
    }

    #[test]
    fn row_detail_update_field_distinguishes_null_and_empty() {
        let dir = temp_dir("update-field");
        let path = dir.join("notes.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT, title TEXT);
                 INSERT INTO notes (body, title) VALUES ('hello', 'hi');",
            )
            .unwrap();
        }
        let db = Db::open(&saved_sqlite("Notes", &path)).unwrap();
        let cell = |sql: &str| db.run_sql(sql, 0).rows[0][0].clone();

        // NULL is not the empty string: the row detail menu offers both.
        assert_eq!(
            db.update_field("notes", "body", "rowid", "1", None).unwrap(),
            1
        );
        assert_eq!(
            cell("SELECT body IS NULL FROM notes WHERE id = 1"),
            Some("1".to_string()),
            "Set NULL must write SQL NULL"
        );
        assert_eq!(
            db.update_field("notes", "body", "rowid", "1", Some("")).unwrap(),
            1
        );
        assert_eq!(
            cell("SELECT body IS NULL FROM notes WHERE id = 1"),
            Some("0".to_string()),
            "Set empty must write an empty string, not NULL"
        );
        assert_eq!(
            cell("SELECT length(body) FROM notes WHERE id = 1"),
            Some("0".to_string())
        );
        assert_eq!(
            db.update_field("notes", "body", "rowid", "1", Some("world"))
                .unwrap(),
            1
        );
        drop(db);

        let conn = Connection::open(&path).unwrap();
        let body: String = conn
            .query_row("SELECT body FROM notes WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(body, "world");
        let title: String = conn
            .query_row("SELECT title FROM notes WHERE id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(title, "hi");
        drop(conn);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn table_and_result_sorting_respects_columns_and_direction() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT, score INTEGER);
             INSERT INTO people (name, score) VALUES ('Mila', 10);
             INSERT INTO people (name, score) VALUES ('Avery', 2);
             INSERT INTO people (name, score) VALUES ('Zoë', 30);",
        )
        .unwrap();

        let asc = vec![SortSpec {
            column: "name".to_string(),
            descending: false,
        }];
        let page = fetch_table(&conn, "people", 100, 0, &TableFilter::default(), &asc);
        let names: Vec<&str> = page
            .rows
            .iter()
            .map(|row| row[1].as_deref().unwrap_or(""))
            .collect();
        assert_eq!(names, ["Avery", "Mila", "Zoë"]);

        let desc = vec![SortSpec {
            column: "name".to_string(),
            descending: true,
        }];
        let page = fetch_table(&conn, "people", 100, 0, &TableFilter::default(), &desc);
        let names: Vec<&str> = page
            .rows
            .iter()
            .map(|row| row[1].as_deref().unwrap_or(""))
            .collect();
        assert_eq!(names, ["Zoë", "Mila", "Avery"]);

        // Unknown columns are ignored by the ORDER BY builder, so the default
        // rowid order stays in place instead of generating invalid SQL.
        let stale = vec![SortSpec {
            column: "missing".to_string(),
            descending: false,
        }];
        let page = fetch_table(&conn, "people", 100, 0, &TableFilter::default(), &stale);
        let names: Vec<&str> = page
            .rows
            .iter()
            .map(|row| row[1].as_deref().unwrap_or(""))
            .collect();
        assert_eq!(names, ["Mila", "Avery", "Zoë"]);
        assert_eq!(page.error, None);

        let columns = vec!["name".to_string(), "score".to_string()];
        let sql = build_order_by(
            &[
                SortSpec {
                    column: "name".to_string(),
                    descending: false,
                },
                SortSpec {
                    column: "score".to_string(),
                    descending: true,
                },
            ],
            &columns,
            Dialect::Sqlite,
        );
        assert_eq!(sql, "ORDER BY \"name\" ASC, \"score\" DESC");
    }

    #[test]
    fn in_memory_sort_is_numeric_and_keeps_nulls_last() {
        let columns = vec!["name".to_string(), "score".to_string()];
        let mut rows = vec![
            vec![Some("b".to_string()), Some("10".to_string())],
            vec![Some("a".to_string()), Some("2".to_string())],
            vec![Some("c".to_string()), None],
            vec![Some("d".to_string()), Some("2".to_string())],
        ];
        let ascending = vec![SortSpec {
            column: "score".to_string(),
            descending: false,
        }];
        sort_rows(&mut rows, &ascending, &columns);
        let names: Vec<&str> = rows
            .iter()
            .map(|row| row[0].as_deref().unwrap_or(""))
            .collect();
        // 2 and 2 tie, then 10; NULL stays last.
        assert_eq!(names, ["a", "d", "b", "c"]);

        let descending = vec![SortSpec {
            column: "score".to_string(),
            descending: true,
        }];
        sort_rows(&mut rows, &descending, &columns);
        let names: Vec<&str> = rows
            .iter()
            .map(|row| row[0].as_deref().unwrap_or(""))
            .collect();
        assert_eq!(names, ["b", "a", "d", "c"]);
    }

    #[test]
    fn table_ids_keep_schema_and_dots_distinct() {
        // A table whose name contains a dot must not be mistaken for a
        // schema-qualified table.
        let dotted = TableId::with_schema(None, "sales.2024");
        assert_eq!(dotted.display(), "sales.2024");
        assert_eq!(dotted.sql(), "sales.2024");
        assert_eq!(split_table_id(&dotted.sql()), (None, "sales.2024"));

        // The same display name, this time as schema "sales" + table "2024",
        // uses the internal qualifier so the SQL builder can tell them apart.
        let qualified = TableId::with_schema(Some("sales".to_string()), "2024");
        assert_eq!(qualified.display(), "sales.2024");
        assert_eq!(table_display_name(&qualified.sql()), "sales.2024");
        assert_eq!(split_table_id(&qualified.sql()), (Some("sales"), "2024"));
    }

    #[test]
    fn postgres_relation_predicates_qualify_only_other_schemas() {
        let bare = TableId::with_schema(None, "users");
        assert_eq!(
            pg_relation_predicate(&bare.sql()),
            ("'users'".to_string(), "current_schema()".to_string())
        );
        let other = TableId::with_schema(Some("sales".to_string()), "users");
        assert_eq!(
            pg_relation_predicate(&other.sql()),
            ("'users'".to_string(), "'sales'".to_string())
        );
    }

    #[test]
    fn sql_dump_filters_are_inline_literals_per_dialect() {
        let columns = vec!["name".to_string()];
        let filter = TableFilter {
            quick: "50%_o'b\\s".to_string(),
            ..TableFilter::default()
        };

        let sqlite = build_where_literals(&filter, &columns, Dialect::Sqlite);
        assert!(sqlite.contains("ESCAPE '\\'"), "{sqlite}");
        assert!(sqlite.contains("o''b"), "{sqlite}");
        assert!(!sqlite.contains('?'), "{sqlite}");

        let mysql = build_where_literals(&filter, &columns, Dialect::MySql);
        assert!(mysql.contains("\\'"), "quote must be backslash-escaped: {mysql}");
        assert!(!mysql.contains('?'), "{mysql}");

        let postgres = build_where_literals(&filter, &columns, Dialect::Postgres);
        assert!(postgres.contains("o''b"), "{postgres}");
        assert!(!postgres.contains('?'), "{postgres}");

        // Column filters also become literals when a dump is built.
        let column_filter = TableFilter {
            columns: vec![ColumnFilter {
                column: "name".to_string(),
                op: FilterOp::Equals,
                value: "O'Brien".to_string(),
            }],
            ..TableFilter::default()
        };
        let sqlite_column = build_where_literals(&column_filter, &columns, Dialect::Sqlite);
        assert!(sqlite_column.contains("o''brien"), "{sqlite_column}");
        assert!(!sqlite_column.contains('?'), "{sqlite_column}");
    }

    #[test]
    fn exports_tables_as_csv_and_xlsx() {
        let dir = temp_dir("table-export");
        let path = dir.join("shop.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT, note TEXT);
                 INSERT INTO people (name, note) VALUES ('A, B', 'line1
line2');
                 INSERT INTO people (name, note) VALUES (NULL, NULL);",
            )
            .unwrap();
        }
        let db = Db::open(&saved_sqlite("Shop", &path)).unwrap();

        let csv = String::from_utf8(
            db.export_table_csv("people", &TableFilter::default(), &[])
                .unwrap(),
        )
        .unwrap();
        assert!(csv.starts_with("\u{feff}id,name,note\r\n"));
        assert!(csv.contains("\"A, B\""), "commas must be quoted: {csv}");
        assert!(csv.contains("\"line1\nline2\""), "newlines must be quoted: {csv}");

        // A filtered export contains every matching row and nothing else.
        let filtered = TableFilter {
            quick: "A, B".to_string(),
            ..TableFilter::default()
        };
        let filtered_csv = String::from_utf8(
            db.export_table_csv("people", &filtered, &[]).unwrap(),
        )
        .unwrap();
        // The quoted value contains a newline, so count the unique cells
        // instead of physical lines.
        assert_eq!(filtered_csv.matches("A, B").count(), 1, "{filtered_csv}");
        assert_eq!(filtered_csv.matches("line1").count(), 1, "{filtered_csv}");
        assert!(!filtered_csv.contains("NULL"));

        let xlsx = db
            .export_table_xlsx("people", &TableFilter::default(), &[])
            .unwrap();
        assert!(xlsx.starts_with(b"PK\x03\x04"));
        std::fs::write("/tmp/dataloom-export-table.xlsx", &xlsx).unwrap();
        drop(db);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn clones_truncates_and_drops_sqlite_tables() {
        let dir = temp_dir("table-ops");
        let path = dir.join("shop.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL);
                 INSERT INTO people (name) VALUES ('Avery');
                 INSERT INTO people (name) VALUES ('Mila');",
            )
            .unwrap();
        }
        let db = Db::open(&saved_sqlite("Shop", &path)).unwrap();

        db.clone_table("people", "people_copy").unwrap();
        let copy = db.fetch_table("people_copy", 0, 0, &TableFilter::default(), &[]);
        assert_eq!(copy.error, None);
        assert_eq!(copy.rows.len(), 2);
        let structure = db.table_structure("people_copy").unwrap();
        assert!(structure.iter().any(|column| column.primary_key));
        assert!(db.clone_table("people", "people_copy").is_err());

        db.truncate_table("people_copy").unwrap();
        let empty = db.fetch_table("people_copy", 0, 0, &TableFilter::default(), &[]);
        assert_eq!(empty.total, 0);
        assert_eq!(empty.rows.len(), 0);

        db.drop_table("people_copy").unwrap();
        assert!(!db
            .list_tables()
            .unwrap()
            .iter()
            .any(|table| table.name == "people_copy"));
        assert!(db.drop_table("people_copy").is_err());
        drop(db);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rewrites_only_the_create_table_name() {
        let plain = rewrite_create_table("CREATE TABLE people (id INTEGER);", "people_copy")
            .unwrap();
        assert_eq!(plain, "CREATE TABLE \"people_copy\" (id INTEGER);");

        let quoted = rewrite_create_table(
            "CREATE TABLE IF NOT EXISTS \"peo\"\"ple\" (id INTEGER);",
            "copy",
        )
        .unwrap();
        assert_eq!(
            quoted,
            "CREATE TABLE IF NOT EXISTS \"copy\" (id INTEGER);"
        );

        let bracketed =
            rewrite_create_table("create table [People] (id integer)", "copy").unwrap();
        assert_eq!(bracketed, "create table \"copy\" (id integer)");
        assert!(rewrite_create_table("not a table", "copy").is_err());
    }

    #[test]
    fn stores_groups_and_keeps_old_connection_files_compatible() {
        // Old stores have neither `groups` nor `group_id`; both default cleanly.
        let old = r#"{"connections":[{"id":1,"name":"Legacy","driver":"SQLite"}],"last_id":1}"#;
        let store: Store = serde_json::from_str(old).unwrap();
        assert!(store.groups.is_empty());
        assert_eq!(store.connections[0].group_id, None);

        let grouped = r#"{
            "connections":[{"id":2,"name":"Demo","driver":"MySQL","group_id":7}],
            "groups":[{"id":7,"name":"OutlookWine"}]
        }"#;
        let store: Store = serde_json::from_str(grouped).unwrap();
        assert_eq!(store.groups[0].name, "OutlookWine");
        assert_eq!(store.connections[0].group_id, Some(7));
        assert_eq!(store.groups[0].id, 7);
    }

    #[test]
    fn column_filters_scope_the_query_to_one_or_more_columns() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT, city TEXT, note TEXT);
             INSERT INTO items (name, city, note) VALUES ('Avery', 'Oslo', NULL);
             INSERT INTO items (name, city, note) VALUES ('Mila', 'Paris', '');
             INSERT INTO items (name, city, note) VALUES ('Zoë', 'Oslo', 'hello');",
        )
        .unwrap();

        let one = |column: &str, op: FilterOp, value: &str| TableFilter {
            columns: vec![ColumnFilter {
                column: column.to_string(),
                op,
                value: value.to_string(),
            }],
            ..TableFilter::default()
        };
        let total = |filter: &TableFilter| fetch_table(&conn, "items", 100, 0, filter, &[]).total;

        // Case-insensitive contains on one column.
        assert_eq!(total(&one("name", FilterOp::Contains, "O")), 1); // Zoë
                                                                     // The Quick Filter alone still searches every column.
        let quick = TableFilter {
            quick: "oslo".to_string(),
            ..TableFilter::default()
        };
        assert_eq!(total(&quick), 2);
        // Several columns, AND by default: Avery from Oslo.
        let and = TableFilter {
            columns: vec![
                ColumnFilter {
                    column: "name".to_string(),
                    op: FilterOp::Contains,
                    value: "av".to_string(),
                },
                ColumnFilter {
                    column: "city".to_string(),
                    op: FilterOp::Equals,
                    value: "OSLO".to_string(),
                },
            ],
            ..TableFilter::default()
        };
        assert_eq!(total(&and), 1);
        // OR mode: Zoë or Paris.
        let or = TableFilter {
            columns: vec![
                ColumnFilter {
                    column: "name".to_string(),
                    op: FilterOp::Contains,
                    value: "ë".to_string(),
                },
                ColumnFilter {
                    column: "city".to_string(),
                    op: FilterOp::Equals,
                    value: "paris".to_string(),
                },
            ],
            any: true,
            ..TableFilter::default()
        };
        assert_eq!(total(&or), 2);
        // The Quick Filter and a column filter combine too.
        let combined = TableFilter {
            quick: "oslo".to_string(),
            columns: vec![ColumnFilter {
                column: "name".to_string(),
                op: FilterOp::StartsWith,
                value: "av".to_string(),
            }],
            ..TableFilter::default()
        };
        assert_eq!(total(&combined), 1);
        // Prefix and suffix.
        assert_eq!(total(&one("name", FilterOp::StartsWith, "mi")), 1);
        assert_eq!(total(&one("name", FilterOp::EndsWith, "la")), 1);
        // Negative filters keep NULL cells visible in a data browser.
        assert_eq!(total(&one("city", FilterOp::NotEquals, "Oslo")), 1); // Mila
        assert_eq!(total(&one("note", FilterOp::NotContains, "hel")), 2); // NULL and ''
                                                                          // Empty / not empty read both NULL and ''.
        assert_eq!(total(&one("note", FilterOp::IsEmpty, "")), 2);
        assert_eq!(total(&one("note", FilterOp::IsNotEmpty, "")), 1);
        // Wildcards are literal, not SQL wildcards.
        assert_eq!(total(&one("name", FilterOp::Contains, "%")), 0);
        // Values are bound, never interpolated into SQL.
        assert_eq!(total(&one("name", FilterOp::Contains, "' OR 1=1 --")), 0);
        // A row without a value and an unknown column do not constrain the query.
        let blank = TableFilter {
            columns: vec![ColumnFilter::new("name")],
            ..TableFilter::default()
        };
        assert!(blank.is_empty());
        assert_eq!(total(&blank), 3);
        assert_eq!(total(&one("missing", FilterOp::Contains, "x")), 3);

        let counted = TableFilter {
            quick: "x".to_string(),
            columns: vec![
                ColumnFilter::new("name"),
                one("city", FilterOp::Contains, "o").columns.remove(0),
            ],
            ..TableFilter::default()
        };
        assert_eq!(counted.active_count(), 2);
    }

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "dataloom-{label}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn saved_sqlite(name: &str, path: &Path) -> SavedConnection {
        SavedConnection {
            id: 1,
            name: name.to_string(),
            path: path.to_string_lossy().to_string(),
            driver: default_driver(),
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

    #[test]
    fn dumps_and_imports_a_table_round_trip() {
        let dir = temp_dir("dump");
        let source = dir.join("source.db");
        {
            let conn = Connection::open(&source).unwrap();
            conn.execute_batch(
                "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT, note TEXT);
                 INSERT INTO people (name, note) VALUES ('O''Brien', NULL);
                 INSERT INTO people (name, note) VALUES ('Zoë', 'line1
line2');",
            )
            .unwrap();
        }
        let db = Db::open(&saved_sqlite("Source", &source)).unwrap();
        let dump = db
            .dump_table_sql("people", &TableFilter::default(), &[])
            .unwrap();
        assert!(dump.contains("CREATE TABLE"));
        assert!(dump.contains("INSERT INTO"));
        // The database quotes the values, so quotes and NULLs survive.
        assert!(dump.contains("'O''Brien'"));
        assert!(dump.contains("NULL"));

        // The same statement with a filter produces only the matching rows.
        let filtered = TableFilter {
            quick: "O'Brien".to_string(),
            ..TableFilter::default()
        };
        let filtered_dump = db.dump_table_sql("people", &filtered, &[]).unwrap();
        assert!(filtered_dump.contains("-- Filtered export"));
        assert!(filtered_dump.contains("'O''Brien'"));
        assert!(!filtered_dump.contains("Zoë"), "filtered dump leaked another row");

        // The filtered script must be valid SQL on its own.
        let filtered_target = dir.join("filtered.db");
        std::fs::write(&filtered_target, b"").unwrap();
        let filtered_db = Db::open(&saved_sqlite("Filtered", &filtered_target)).unwrap();
        assert_eq!(filtered_db.import_sql(&filtered_dump).unwrap(), 2);
        let filtered_page =
            filtered_db.fetch_table("people", 10, 0, &TableFilter::default(), &[]);
        assert_eq!(filtered_page.rows.len(), 1);
        assert_eq!(filtered_page.rows[0][1].as_deref(), Some("O'Brien"));

        let target = dir.join("target.db");
        std::fs::write(&target, b"").unwrap();
        let target_db = Db::open(&saved_sqlite("Target", &target)).unwrap();
        let executed = target_db.import_sql(&dump).unwrap();
        assert_eq!(executed, 3);

        let page = target_db.fetch_table("people", 100, 0, &TableFilter::default(), &[]);
        assert_eq!(page.total, 2);
        assert_eq!(page.rows[0][1].as_deref(), Some("O'Brien"));
        assert_eq!(page.rows[0][2], None);
        assert_eq!(page.rows[1][1].as_deref(), Some("Zoë"));
        assert_eq!(page.rows[1][2].as_deref(), Some("line1\nline2"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn dumps_the_whole_database_with_views() {
        let dir = temp_dir("dump-db");
        let path = dir.join("shop.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT);
                 INSERT INTO people (name) VALUES ('Ada');
                 CREATE VIEW names AS SELECT name FROM people;",
            )
            .unwrap();
        }
        let db = Db::open(&saved_sqlite("Shop", &path)).unwrap();
        let dump = db.dump_database_sql("Shop").unwrap();
        assert!(dump.contains("-- Database: Shop"));
        assert!(dump.contains("Engine: SQLite"));
        assert!(dump.contains("-- Table: people"));
        assert!(dump.contains("-- View: names"));
        assert!(dump.contains("INSERT INTO"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn whole_database_dump_round_trips() {
        let dir = temp_dir("roundtrip");
        let source = dir.join("source.db");
        {
            let conn = Connection::open(&source).unwrap();
            conn.execute_batch(
                "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT);
                 INSERT INTO people (name) VALUES ('Ada');
                 INSERT INTO people (name) VALUES ('Grace');
                 CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT);
                 INSERT INTO notes (body) VALUES ('hi');
                 CREATE VIEW names AS SELECT name FROM people;",
            )
            .unwrap();
        }
        let db = Db::open(&saved_sqlite("Source", &source)).unwrap();
        let dump = db.dump_database_sql("Source").unwrap();

        let target = dir.join("target.db");
        std::fs::write(&target, b"").unwrap();
        let target_db = Db::open(&saved_sqlite("Target", &target)).unwrap();
        target_db.import_sql(&dump).unwrap();

        let people = target_db.fetch_table("people", 100, 0, &TableFilter::default(), &[]);
        assert_eq!(people.total, 2);
        let notes = target_db.fetch_table("notes", 100, 0, &TableFilter::default(), &[]);
        assert_eq!(notes.total, 1);
        // The view is recreated too.
        let built = target_db.run_sql("SELECT name FROM names ORDER BY name", 100);
        assert_eq!(built.error, None);
        assert_eq!(built.rows.len(), 2);
        assert_eq!(built.rows[0][0].as_deref(), Some("Ada"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn backs_up_sqlite_files_and_lists_them() {
        let dir = temp_dir("backup");
        let path = dir.join("shop.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE t (id INTEGER); INSERT INTO t VALUES (1);")
                .unwrap();
        }
        let saved = saved_sqlite("Sample shop", &path);
        let db = Db::open(&saved).unwrap();
        let backup_dir = dir.join("backups");

        let backup = create_backup_in(&backup_dir, &saved, &db).unwrap();
        assert!(backup.exists());
        assert_eq!(
            std::fs::metadata(&backup).unwrap().len(),
            std::fs::metadata(&path).unwrap().len()
        );

        let list = list_backups_in(&backup_dir);
        assert_eq!(list.len(), 1);
        assert!(list[0].name.ends_with(".db"));
        assert!(list[0].bytes > 0);

        delete_backup(&backup).unwrap();
        assert!(list_backups_in(&backup_dir).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn file_name_helpers_are_safe() {
        assert_eq!(sanitize_file_stem("Sample shop"), "Sample-shop");
        assert_eq!(sanitize_file_stem("a/b..c"), "a-b-c");
        assert_eq!(sanitize_file_stem("  "), "dataloom");
        assert_eq!(utc_parts(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(utc_parts(1_700_000_000), (2023, 11, 14, 22, 13, 20));
    }

    #[test]
    fn inserts_rows_and_changes_columns() {
        let path = std::env::temp_dir().join(format!(
            "dataloom-structure-test-{}-{:?}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT);")
            .unwrap();
        drop(conn);

        let saved = SavedConnection {
            id: 1,
            name: "test".to_string(),
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
        };
        let db = Db::open(&saved).unwrap();

        db.insert_row("people", &[("name".to_string(), "Avery".to_string())])
            .unwrap();
        db.add_column("people", "city", "TEXT", false, "").unwrap();
        assert!(db
            .table_structure("people")
            .unwrap()
            .iter()
            .any(|c| c.name == "city"));

        db.rename_column("people", "city", "town").unwrap();
        assert!(db
            .table_structure("people")
            .unwrap()
            .iter()
            .any(|c| c.name == "town"));

        db.drop_column("people", "town").unwrap();
        let page = db.fetch_table("people", 100, 0, &TableFilter::default(), &[]);
        assert_eq!(page.total, 1);
        assert_eq!(page.rows[0][1].as_deref(), Some("Avery"));

        db.insert_row("people", &[("name".to_string(), "Mila".to_string())])
            .unwrap();
        let page = db.fetch_table("people", 100, 0, &TableFilter::default(), &[]);
        assert_eq!(page.total, 2);
        let key_column = page.key_column.clone().unwrap();
        let key_value = page.rowids[0].clone().unwrap();
        assert_eq!(db.delete_row("people", &key_column, &key_value).unwrap(), 1);
        assert_eq!(
            db.fetch_table("people", 100, 0, &TableFilter::default(), &[])
                .total,
            1
        );
        // Deleting the same key again removes nothing.
        assert_eq!(db.delete_row("people", &key_column, &key_value).unwrap(), 0);

        drop(db);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn paging_rules_match_sub_10071b5e4() {
        assert_eq!(
            validate_paging(0, 0, 100),
            Err("The limitation should be greater than zero")
        );
        assert_eq!(
            validate_paging(10_000_001, 0, 100),
            Err("The limitation is too large")
        );
        assert_eq!(
            validate_paging(100, -1, 100),
            Err("The offset should be positive")
        );
        assert_eq!(
            validate_paging(100, 100, 100),
            Err("The offset should be smaller than the total of rows")
        );
        // `sub_10071b5e4` rejects any non-zero offset >= total; an empty
        // result therefore rejects offset 1.
        assert_eq!(
            validate_paging(100, 1, 0),
            Err("The offset should be smaller than the total of rows")
        );
        assert!(validate_paging(100, 0, 100).is_ok());
        assert!(validate_paging(100, 0, 0).is_ok());
        assert!(validate_paging(10_000_000, 0, 0).is_ok());
    }

    #[test]
    fn paging_text_matches_sub_100532a80() {
        assert_eq!(parse_paging_text("42"), 42);
        assert_eq!(parse_paging_text("+42"), 42);
        assert_eq!(parse_paging_text("-7"), -7);
        // `sub_100532a80` accepts only an optional sign and digits; it does
        // not trim whitespace or accept a partial number.
        assert_eq!(parse_paging_text(" 42"), 0);
        assert_eq!(parse_paging_text("42 "), 0);
        assert_eq!(parse_paging_text("4 2"), 0);
        assert_eq!(parse_paging_text("abc"), 0);
    }

    #[test]
    fn page_step_matches_sub_10071b144_and_sub_10071b370() {
        // `csel x, 0x12c, limit, lt`: 300 is used whenever the stored limit
        // is lower, including "No limit" (0).
        assert_eq!(page_step(0), 300);
        assert_eq!(page_step(100), 300);
        assert_eq!(page_step(300), 300);
        assert_eq!(page_step(500), 500);
        assert_eq!(DEFAULT_TABLE_PAGE_LIMIT, 300);
    }
}
