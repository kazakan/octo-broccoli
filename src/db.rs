use anyhow::{bail, Context, Result};
use rusqlite::{types::ValueRef, Connection};
use serde_json::{Map, Value};

use crate::config::Source;

/// Query SQLite and return every row as a JSON object map.
pub fn query_rows(source: &Source, db_path_override: Option<&str>) -> Result<Vec<Map<String, Value>>> {
    let db_path = db_path_override.unwrap_or(&source.db_path);
    let conn = Connection::open(db_path)
        .with_context(|| format!("opening SQLite database '{db_path}'"))?;

    let sql = build_sql(source)?;
    let mut stmt = conn
        .prepare(&sql)
        .with_context(|| format!("preparing SQL: {sql}"))?;

    let col_names: Vec<String> = stmt
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect();

    let rows = stmt
        .query_map([], |row| {
            let mut map = Map::new();
            for (i, col) in col_names.iter().enumerate() {
                let val = sqlite_value_to_json(row.get_ref(i)?);
                map.insert(col.clone(), val);
            }
            Ok(map)
        })
        .context("executing SQL query")?
        .collect::<Result<Vec<_>, _>>()
        .context("fetching rows")?;

    Ok(rows)
}

fn build_sql(source: &Source) -> Result<String> {
    if let Some(ref q) = source.query {
        return Ok(q.clone());
    }
    if let Some(ref table) = source.table {
        return Ok(format!("SELECT * FROM \"{table}\""));
    }
    bail!("source must specify either 'query' or 'table'");
}

fn sqlite_value_to_json(v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::Number(i.into()),
        ValueRef::Real(f) => serde_json::Number::from_f64(f)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Value::String(format!("<blob {} bytes>", b.len())),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::path::PathBuf;

    /// Create a temp SQLite file with a `users` table and a few rows.
    /// Returns the file path; the file is removed when the returned `_guard`
    /// is dropped (via a simple wrapper).
    fn create_test_db() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "ocbro_test_{}.db",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE users (id INTEGER, name TEXT, age INTEGER, deleted_at TEXT);
             INSERT INTO users VALUES (1, 'Alice', 30, NULL);
             INSERT INTO users VALUES (2, 'Bob',   17, NULL);
             INSERT INTO users VALUES (3, 'Carol', 25, '2024-01-01');",
        )
        .unwrap();
        path
    }

    #[test]
    fn test_query_rows_by_table() {
        let db = create_test_db();
        let source = Source {
            db_path: db.display().to_string(),
            table: Some("users".to_string()),
            query: None,
        };
        let rows = query_rows(&source, None).unwrap();
        assert_eq!(rows.len(), 3);
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn test_query_rows_by_raw_sql() {
        let db = create_test_db();
        let source = Source {
            db_path: db.display().to_string(),
            table: None,
            query: Some("SELECT * FROM users WHERE age >= 18".to_string()),
        };
        let rows = query_rows(&source, None).unwrap();
        assert_eq!(rows.len(), 2);
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn test_query_rows_db_path_override() {
        let db = create_test_db();
        // source.db_path points somewhere else, but override wins.
        let source = Source {
            db_path: "/nonexistent.db".to_string(),
            table: Some("users".to_string()),
            query: None,
        };
        let rows = query_rows(&source, Some(db.to_str().unwrap())).unwrap();
        assert_eq!(rows.len(), 3);
        let _ = std::fs::remove_file(&db);
    }

    #[test]
    fn test_query_rows_column_types() {
        let db = create_test_db();
        let source = Source {
            db_path: db.display().to_string(),
            table: Some("users".to_string()),
            query: None,
        };
        let rows = query_rows(&source, None).unwrap();
        // Alice: id=1 (int), name="Alice" (string), deleted_at=NULL
        let alice = rows.iter().find(|r| r["name"] == "Alice").unwrap();
        assert_eq!(alice["id"], serde_json::json!(1));
        assert_eq!(alice["deleted_at"], serde_json::Value::Null);
        let _ = std::fs::remove_file(&db);
    }
}
