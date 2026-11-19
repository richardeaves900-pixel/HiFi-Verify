use rusqlite::{Connection, Result};
use std::path::PathBuf;

pub fn get_db_path() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("hifi-verify");
    std::fs::create_dir_all(&path).ok();
    path.push("history.db");
    path
}

fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS xruns (
            id INTEGER PRIMARY KEY,
            timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
            count INTEGER NOT NULL,
            node_name TEXT NOT NULL DEFAULT 'system'
        )",
        [],
    )?;
    // Upgrade pre-existing databases that lacked the node_name column.
    let has_node_col: bool = conn
        .prepare("PRAGMA table_info(xruns)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .flatten()
        .any(|name| name == "node_name");
    if !has_node_col {
        let _ = conn.execute(
            "ALTER TABLE xruns ADD COLUMN node_name TEXT NOT NULL DEFAULT 'system'",
            [],
        );
    }
    Ok(())
}

pub fn init_db() -> Result<Connection> {
    let conn = Connection::open(get_db_path())?;
    ensure_schema(&conn)?;
    Ok(conn)
}

pub fn log_node_xruns(node_name: &str, count: u64) {
    if count == 0 {
        return;
    }
    if let Ok(conn) = init_db() {
        let _ = conn.execute(
            "INSERT INTO xruns (count, node_name) VALUES (?1, ?2)",
            rusqlite::params![count as i64, node_name],
        );
    }
}

pub fn format_history_summary(conn: &Connection) -> String {
    let mut summary = String::new();
    let mut stmt = match conn
        .prepare("SELECT timestamp, count, node_name FROM xruns ORDER BY id DESC LIMIT 50")
    {
        Ok(stmt) => stmt,
        Err(err) => return format!("Failed to prepare XRUN history: {err}"),
    };
    let rows = match stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
        ))
    }) {
        Ok(rows) => rows,
        Err(err) => return format!("Failed to query XRUN history: {err}"),
    };
    summary.push_str("--- XRUN History (Last 50) ---\n\n");
    let mut found = false;
    for (ts, count, node) in rows.flatten() {
        summary.push_str(&format!("{ts} | +{count} XRUN(s) | {node}\n"));
        found = true;
    }
    if !found {
        summary.push_str("No new XRUNs logged yet.");
    }
    summary
}

pub fn get_history_summary() -> String {
    match init_db() {
        Ok(conn) => format_history_summary(&conn),
        Err(err) => format!("Failed to open XRUN history: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_legacy_table_and_formats_node_entries() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE xruns (id INTEGER PRIMARY KEY, timestamp DATETIME DEFAULT CURRENT_TIMESTAMP, count INTEGER NOT NULL)",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO xruns (count) VALUES (3)", [])
            .unwrap();

        ensure_schema(&conn).unwrap();
        conn.execute(
            "INSERT INTO xruns (count, node_name) VALUES (2, 'alsa_output.usb-Fiio')",
            [],
        )
        .unwrap();

        let text = format_history_summary(&conn);
        assert!(text.contains("+2 XRUN(s) | alsa_output.usb-Fiio"));
        assert!(text.contains("+3 XRUN(s) | system"));
    }
}
