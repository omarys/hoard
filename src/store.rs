//! SQLite persistence for the command trove.
//!
//! The trove is kept fully in memory while the interactive or CLI flows run
//! (the GUI needs random access and per-keystroke filtering), and synced to
//! SQLite whenever the in-memory state changes. The connection is tuned for
//! low-latency single-user writes: WAL journaling, `NORMAL` synchronous mode
//! and prepared statements inside one transaction per sync.
//!
//! Commands are stored in a `STRICT` table keyed by `(namespace, name)`;
//! timestamps are unix seconds. A fresh database transparently imports a
//! legacy `trove.yml` sitting next to it so existing users keep their data.

use crate::core::HoardCmd;
use anyhow::{Context, Result};
use rusqlite::{Connection, Row, backup::Backup, params};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Name of the SQLite database file inside the hoard config directory.
pub const TROVE_DB: &str = "trove.db";
/// Legacy YAML trove file. Imported once when a new database is created.
pub const LEGACY_TROVE_FILE: &str = "trove.yml";

const SCHEMA_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS commands (
    name         TEXT NOT NULL,
    namespace    TEXT NOT NULL,
    command      TEXT NOT NULL,
    description  TEXT NOT NULL DEFAULT '',
    tags         TEXT NOT NULL DEFAULT '',
    created      INTEGER NOT NULL,
    modified     INTEGER NOT NULL,
    last_used    INTEGER NOT NULL,
    usage_count  INTEGER NOT NULL DEFAULT 0,
    is_favorite  INTEGER NOT NULL DEFAULT 0,
    is_hidden    INTEGER NOT NULL DEFAULT 0,
    is_deleted   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (namespace, name)
) STRICT;
CREATE INDEX IF NOT EXISTS idx_commands_namespace ON commands(namespace);
";

const INSERT_SQL: &str = "
INSERT INTO commands (
    name, namespace, command, description, tags,
    created, modified, last_used, usage_count,
    is_favorite, is_hidden, is_deleted
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
";

/// A connection to the trove database.
pub struct Db {
    conn: Connection,
}

impl Db {
    /// Opens (creating if needed) the database at `path`, runs migrations and
    /// imports a legacy `trove.yml` next to it when the database is brand new.
    pub fn open(path: &Path) -> Result<Self> {
        let is_new = !path.exists();
        let conn = Connection::open(path)
            .with_context(|| format!("Could not open trove database {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(Duration::from_secs(5))?;
        migrate(&conn)?;

        let db = Self { conn };
        if is_new {
            db.import_legacy_yaml(path)?;
        }
        Ok(db)
    }

    /// Loads every command from the database.
    pub fn commands(&self) -> Result<Vec<HoardCmd>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT * FROM commands ORDER BY rowid")?;
        let commands = stmt
            .query_map([], row_to_command)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(commands)
    }

    /// Replaces the database contents with `commands` in one transaction.
    pub fn sync(&self, commands: &[HoardCmd]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM commands", [])?;
        {
            let mut stmt = tx.prepare_cached(INSERT_SQL)?;
            for command in commands {
                stmt.execute(params![
                    command.name,
                    command.namespace,
                    command.command,
                    command.description,
                    command.get_tags_as_string(),
                    to_unix(command.created),
                    to_unix(command.modified),
                    to_unix(command.last_used),
                    command.usage_count as i64,
                    i64::from(command.is_favorite),
                    i64::from(command.is_hidden),
                    i64::from(command.is_deleted),
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Writes a consistent snapshot of the database to `dest`.
    pub fn backup_to(&self, dest: &Path) -> Result<()> {
        let mut dest_conn = Connection::open(dest)?;
        let backup = Backup::new(&self.conn, &mut dest_conn)?;
        backup.step(-1)?;
        Ok(())
    }

    /// Imports a legacy `trove.yml` (a sibling of the database file) when the
    /// database is empty. Keeps the YAML file untouched.
    fn import_legacy_yaml(&self, db_path: &Path) -> Result<()> {
        let legacy = db_path.with_extension("yml");
        if !legacy.exists() {
            return Ok(());
        }
        match crate::core::trove::Trove::from_yaml_file(&legacy) {
            Ok(trove) => self.sync(&trove.commands)?,
            Err(err) => {
                // A broken legacy file must not brick the new database.
                eprintln!(
                    "Warning: could not import legacy trove file {}: {err}",
                    legacy.display()
                );
            }
        }
        Ok(())
    }
}

/// Returns `true` when a local (current-directory) trove exists, either the
/// new database or the legacy YAML file.
pub fn local_trove_exists() -> bool {
    Path::new(TROVE_DB).exists() || Path::new(LEGACY_TROVE_FILE).exists()
}

fn migrate(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version < SCHEMA_VERSION {
        conn.execute_batch(SCHEMA)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    Ok(())
}

fn row_to_command(row: &Row) -> rusqlite::Result<HoardCmd> {
    let tags: String = row.get("tags")?;
    Ok(HoardCmd {
        name: row.get("name")?,
        command: row.get("command")?,
        description: row.get("description")?,
        tags: tags
            .split(',')
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect(),
        created: from_unix(row.get("created")?),
        modified: from_unix(row.get("modified")?),
        last_used: from_unix(row.get("last_used")?),
        usage_count: row.get::<_, i64>("usage_count")? as usize,
        is_favorite: row.get::<_, i64>("is_favorite")? != 0,
        is_hidden: row.get::<_, i64>("is_hidden")? != 0,
        is_deleted: row.get::<_, i64>("is_deleted")? != 0,
        namespace: row.get("namespace")?,
    })
}

fn to_unix(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn from_unix(secs: i64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(secs.max(0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::HoardCmd;
    use std::fs;

    fn command(name: &str, namespace: &str, command: &str) -> HoardCmd {
        HoardCmd::default()
            .with_command(command)
            .with_namespace(namespace)
            .with_name(name)
    }

    #[test]
    fn roundtrips_commands() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join(TROVE_DB)).unwrap();
        let commands = vec![
            command("cmd1", "ns1", "echo one"),
            command("cmd2", "ns2", "echo two"),
        ];
        db.sync(&commands).unwrap();
        assert_eq!(db.commands().unwrap(), commands);
    }

    #[test]
    fn sync_replaces_contents() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join(TROVE_DB)).unwrap();
        db.sync(&[command("a", "ns", "echo a")]).unwrap();
        db.sync(&[command("b", "ns", "echo b")]).unwrap();
        let loaded = db.commands().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "b");
    }

    #[test]
    fn imports_legacy_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = dir.path().join(LEGACY_TROVE_FILE);
        fs::write(
            &legacy,
            "version: 2.0.0\ncommands:\n  - name: legacy\n    namespace: default\n    command: echo legacy\n    description: A legacy import\n",
        )
        .unwrap();
        let db = Db::open(&dir.path().join(TROVE_DB)).unwrap();
        let commands = db.commands().unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "legacy");
        assert_eq!(commands[0].command, "echo legacy");
    }

    #[test]
    fn ignores_missing_legacy_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join(TROVE_DB)).unwrap();
        assert!(db.commands().unwrap().is_empty());
    }

    #[test]
    fn backup_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join(TROVE_DB)).unwrap();
        db.sync(&[command("a", "ns", "echo a")]).unwrap();
        let backup_path = dir.path().join("trove.db.bk");
        db.backup_to(&backup_path).unwrap();
        let backup = Db::open(&backup_path).unwrap();
        assert_eq!(backup.commands().unwrap().len(), 1);
    }

    #[test]
    fn timestamps_roundtrip() {
        let now = SystemTime::now();
        let restored = from_unix(to_unix(now));
        let skew = restored
            .duration_since(now)
            .unwrap_or_else(|e| e.duration())
            .as_secs();
        assert!(skew <= 1);
        assert_eq!(to_unix(UNIX_EPOCH), 0);
    }
}
