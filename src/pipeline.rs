use serde_json::{Map, Value};

use crate::config::{FilterCondition, FilterOp};

/// Apply all filter conditions (AND logic) to a row.
/// Returns `true` if the row should be kept.
pub fn apply_filter(row: &Map<String, Value>, conditions: &[FilterCondition]) -> bool {
    conditions.iter().all(|cond| matches_condition(row, cond))
}

fn matches_condition(row: &Map<String, Value>, cond: &FilterCondition) -> bool {
    let cell = row.get(&cond.field).unwrap_or(&Value::Null);

    match &cond.op {
        FilterOp::IsNull => cell.is_null(),
        FilterOp::IsNotNull => !cell.is_null(),

        FilterOp::Eq => json_eq(cell, cond.value.as_ref().unwrap_or(&Value::Null)),
        FilterOp::Ne => !json_eq(cell, cond.value.as_ref().unwrap_or(&Value::Null)),

        FilterOp::Gt => numeric_cmp(cell, cond.value.as_ref()) == Some(std::cmp::Ordering::Greater),
        FilterOp::Lt => numeric_cmp(cell, cond.value.as_ref()) == Some(std::cmp::Ordering::Less),
        FilterOp::Gte => matches!(
            numeric_cmp(cell, cond.value.as_ref()),
            Some(std::cmp::Ordering::Greater) | Some(std::cmp::Ordering::Equal)
        ),
        FilterOp::Lte => matches!(
            numeric_cmp(cell, cond.value.as_ref()),
            Some(std::cmp::Ordering::Less) | Some(std::cmp::Ordering::Equal)
        ),

        FilterOp::Contains => string_op(cell, cond.value.as_ref(), |s, pat| s.contains(pat)),
        FilterOp::StartsWith => string_op(cell, cond.value.as_ref(), |s, pat| s.starts_with(pat)),
        FilterOp::EndsWith => string_op(cell, cond.value.as_ref(), |s, pat| s.ends_with(pat)),
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn json_eq(a: &Value, b: &Value) -> bool {
    // Try numeric comparison first to handle integer vs float edge cases.
    if let (Some(an), Some(bn)) = (as_f64(a), as_f64(b)) {
        return an == bn;
    }
    a == b
}

fn as_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

fn numeric_cmp(cell: &Value, target: Option<&Value>) -> Option<std::cmp::Ordering> {
    let a = as_f64(cell)?;
    let b = as_f64(target?)?;
    a.partial_cmp(&b)
}

fn string_op(cell: &Value, target: Option<&Value>, f: impl Fn(&str, &str) -> bool) -> bool {
    let s = match cell {
        Value::String(s) => s.as_str(),
        _ => return false,
    };
    let pat = match target {
        Some(Value::String(p)) => p.as_str(),
        _ => return false,
    };
    f(s, pat)
}

// ── Projection ───────────────────────────────────────────────────────────────

/// Keep only the requested fields. If `fields` is empty, return the row unchanged.
pub fn apply_projection(row: Map<String, Value>, fields: &[String]) -> Map<String, Value> {
    if fields.is_empty() {
        return row;
    }
    fields
        .iter()
        .filter_map(|k| {
            let v = row.get(k)?.clone();
            Some((k.clone(), v))
        })
        .collect()
}
