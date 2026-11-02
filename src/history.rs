use rusqlite::{Connection, Result};
use std::path::PathBuf;

pub fn get_db_path() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("hifi-verify");
    std::fs::create_dir_all(&path).ok();
    path.push("history.db");
    path
}

pub fn init_db() -> Result<Connection> {
    let conn = Connection::open(get_db_path())?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS xruns (
            id INTEGER PRIMARY KEY,
            timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
            count INTEGER NOT NULL
        )",
        [],
    )?;
    Ok(conn)
}

pub fn log_xruns(count: u64) {
    if count == 0 { return; }
    if let Ok(conn) = init_db() {
        let _ = conn.execute("INSERT INTO xruns (count) VALUES (?1)", [count as i64]);
    }
}

pub fn get_history_summary() -> String {
    let mut summary = String::new();
    match init_db() {
        Ok(conn) => {
            let mut stmt = conn.prepare("SELECT timestamp, count FROM xruns ORDER BY timestamp DESC LIMIT 50").unwrap();
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            }).unwrap();

            summary.push_str("--- XRUN History (Last 50) ---\n\n");
            let mut found = false;
            for row in rows.flatten() {
                summary.push_str(&format!("{} | {} dropped frames\n", row.0, row.1));
                found = true;
            }
            if !found {
                summary.push_str("No XRUNs logged yet. Your audio is pristine!");
            }
        }
        Err(e) => summary.push_str(&format!("Failed to read database: {}", e)),
    }
    summary
}
