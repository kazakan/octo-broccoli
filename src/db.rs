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
