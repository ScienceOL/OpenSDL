use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

use crate::event::OsdlEvent;
use crate::protocol::DeviceCommand;

/// Append-only local event store for safety auditing and forensic replay.
///
/// Uses SQLite in WAL mode for crash-safe writes. All data is append-only —
/// nothing is ever deleted by the engine (rotation is the operator's choice).
pub struct EventStore {
    conn: Mutex<Connection>,
}

impl EventStore {
    /// Open (or create) the SQLite database at the given path.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("sqlite open: {}", e))?;

        // WAL mode for concurrent reads + crash safety
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
            .map_err(|e| format!("sqlite pragma: {}", e))?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS event_log (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp   INTEGER NOT NULL,
                event_type  TEXT NOT NULL,
                payload     TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS command_log (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp   INTEGER NOT NULL,
                command_id  TEXT NOT NULL,
                device_id   TEXT NOT NULL,
                action      TEXT NOT NULL,
                params      TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS serial_log (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp   INTEGER NOT NULL,
                node_id     TEXT NOT NULL,
                direction   TEXT NOT NULL,
                bytes       BLOB NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_event_ts ON event_log(timestamp);
            CREATE INDEX IF NOT EXISTS idx_command_device ON command_log(device_id, timestamp);
            CREATE INDEX IF NOT EXISTS idx_serial_node ON serial_log(node_id, timestamp);
            ",
        )
        .map_err(|e| format!("sqlite schema: {}", e))?;

        log::info!("Event store opened");
        Ok(EventStore {
            conn: Mutex::new(conn),
        })
    }

    /// Open an in-memory database (for testing).
    pub fn in_memory() -> Result<Self, String> {
        Self::open(":memory:")
    }

    /// Log an OsdlEvent.
    pub fn log_event(&self, event: &OsdlEvent) {
        let now = now_millis();
        let event_type = event_type_str(event);
        let payload = serde_json::to_string(event).unwrap_or_default();

        let conn = self.conn.lock().unwrap();
        if let Err(e) = conn.execute(
            "INSERT INTO event_log (timestamp, event_type, payload) VALUES (?1, ?2, ?3)",
            params![now, event_type, payload],
        ) {
            log::error!("Failed to log event: {}", e);
        }
    }

    /// Log an outgoing command.
    pub fn log_command(&self, cmd: &DeviceCommand) {
        let now = now_millis();
        let params_json = serde_json::to_string(&cmd.params).unwrap_or_default();

        let conn = self.conn.lock().unwrap();
        if let Err(e) = conn.execute(
            "INSERT INTO command_log (timestamp, command_id, device_id, action, params) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![now, cmd.command_id, cmd.device_id, cmd.action, params_json],
        ) {
            log::error!("Failed to log command: {}", e);
        }
    }

    /// Log raw serial bytes (tx or rx).
    pub fn log_serial(&self, node_id: &str, direction: &str, bytes: &[u8]) {
        let now = now_millis();

        let conn = self.conn.lock().unwrap();
        if let Err(e) = conn.execute(
            "INSERT INTO serial_log (timestamp, node_id, direction, bytes) VALUES (?1, ?2, ?3, ?4)",
            params![now, node_id, direction, bytes],
        ) {
            log::error!("Failed to log serial: {}", e);
        }
    }

    /// Query events by type and time range. Returns JSON payloads.
    pub fn query_events(
        &self,
        event_type: Option<&str>,
        since: Option<i64>,
        until: Option<i64>,
        limit: usize,
    ) -> Vec<StoredEvent> {
        let conn = self.conn.lock().unwrap();
        let mut sql =
            String::from("SELECT id, timestamp, event_type, payload FROM event_log WHERE 1=1");
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(t) = event_type {
            sql.push_str(" AND event_type = ?");
            param_values.push(Box::new(t.to_string()));
        }
        if let Some(s) = since {
            sql.push_str(" AND timestamp >= ?");
            param_values.push(Box::new(s));
        }
        if let Some(u) = until {
            sql.push_str(" AND timestamp <= ?");
            param_values.push(Box::new(u));
        }
        sql.push_str(" ORDER BY timestamp DESC LIMIT ?");
        param_values.push(Box::new(limit as i64));

        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let mut stmt = match conn.prepare(&sql) {
            Ok(s) => s,
            Err(e) => {
                log::error!("Failed to prepare query: {}", e);
                return Vec::new();
            }
        };

        let rows = stmt
            .query_map(params_refs.as_slice(), |row| {
                Ok(StoredEvent {
                    id: row.get(0)?,
                    timestamp: row.get(1)?,
                    event_type: row.get(2)?,
                    payload: row.get(3)?,
                })
            })
            .unwrap_or_else(|e| {
                log::error!("Failed to query events: {}", e);
                // Return empty iterator by querying with impossible condition
                panic!("query failed: {}", e);
            });

        rows.filter_map(|r| r.ok()).collect()
    }

    /// Query serial log for a specific node. For forensic replay.
    pub fn query_serial(
        &self,
        node_id: &str,
        since: Option<i64>,
        limit: usize,
    ) -> Vec<StoredSerial> {
        let conn = self.conn.lock().unwrap();
        let since = since.unwrap_or(0);

        let mut stmt = conn
            .prepare(
                "SELECT id, timestamp, node_id, direction, bytes FROM serial_log
                 WHERE node_id = ?1 AND timestamp >= ?2
                 ORDER BY timestamp ASC LIMIT ?3",
            )
            .unwrap();

        stmt.query_map(params![node_id, since, limit as i64], |row| {
            Ok(StoredSerial {
                id: row.get(0)?,
                timestamp: row.get(1)?,
                node_id: row.get(2)?,
                direction: row.get(3)?,
                bytes: row.get(4)?,
            })
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
    }
}

#[derive(Debug, Clone)]
pub struct StoredEvent {
    pub id: i64,
    pub timestamp: i64,
    pub event_type: String,
    pub payload: String,
}

#[derive(Debug, Clone)]
pub struct StoredSerial {
    pub id: i64,
    pub timestamp: i64,
    pub node_id: String,
    pub direction: String,
    pub bytes: Vec<u8>,
}

fn event_type_str(event: &OsdlEvent) -> &'static str {
    match event {
        OsdlEvent::DeviceOnline(_) => "device_online",
        OsdlEvent::DeviceOffline { .. } => "device_offline",
        OsdlEvent::DeviceStatus(_) => "device_status",
        OsdlEvent::CommandResult(_) => "command_result",
        OsdlEvent::UnknownNode { .. } => "unknown_node",
    }
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
