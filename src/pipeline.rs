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

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{FilterCondition, FilterOp};
    use serde_json::json;

    fn row(pairs: &[(&str, serde_json::Value)]) -> Map<String, Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    fn cond(field: &str, op: FilterOp, value: Option<Value>) -> FilterCondition {
        FilterCondition { field: field.to_string(), op, value }
    }

    // ── filter: eq / ne ──────────────────────────────────────────────────────

    #[test]
    fn test_filter_eq_int_match() {
        let r = row(&[("age", json!(18))]);
        assert!(apply_filter(&r, &[cond("age", FilterOp::Eq, Some(json!(18)))]));
    }

    #[test]
    fn test_filter_eq_int_no_match() {
        let r = row(&[("age", json!(17))]);
        assert!(!apply_filter(&r, &[cond("age", FilterOp::Eq, Some(json!(18)))]));
    }

    #[test]
    fn test_filter_eq_string() {
        let r = row(&[("name", json!("Alice"))]);
        assert!(apply_filter(&r, &[cond("name", FilterOp::Eq, Some(json!("Alice")))]));
        assert!(!apply_filter(&r, &[cond("name", FilterOp::Eq, Some(json!("Bob")))]));
    }

    #[test]
    fn test_filter_ne() {
        let r = row(&[("age", json!(20))]);
        assert!(apply_filter(&r, &[cond("age", FilterOp::Ne, Some(json!(18)))]));
        assert!(!apply_filter(&r, &[cond("age", FilterOp::Ne, Some(json!(20)))]));
    }

    // ── filter: numeric comparisons ──────────────────────────────────────────

    #[test]
    fn test_filter_gt() {
        let r = row(&[("age", json!(19))]);
        assert!(apply_filter(&r, &[cond("age", FilterOp::Gt, Some(json!(18)))]));
        assert!(!apply_filter(&r, &[cond("age", FilterOp::Gt, Some(json!(19)))]));
        assert!(!apply_filter(&r, &[cond("age", FilterOp::Gt, Some(json!(20)))]));
    }

    #[test]
    fn test_filter_lt() {
        let r = row(&[("age", json!(17))]);
        assert!(apply_filter(&r, &[cond("age", FilterOp::Lt, Some(json!(18)))]));
        assert!(!apply_filter(&r, &[cond("age", FilterOp::Lt, Some(json!(17)))]));
    }

    #[test]
    fn test_filter_gte() {
        let r = row(&[("age", json!(18))]);
        assert!(apply_filter(&r, &[cond("age", FilterOp::Gte, Some(json!(18)))]));
        assert!(apply_filter(&r, &[cond("age", FilterOp::Gte, Some(json!(17)))]));
        assert!(!apply_filter(&r, &[cond("age", FilterOp::Gte, Some(json!(19)))]));
    }

    #[test]
    fn test_filter_lte() {
        let r = row(&[("age", json!(18))]);
        assert!(apply_filter(&r, &[cond("age", FilterOp::Lte, Some(json!(18)))]));
        assert!(apply_filter(&r, &[cond("age", FilterOp::Lte, Some(json!(19)))]));
        assert!(!apply_filter(&r, &[cond("age", FilterOp::Lte, Some(json!(17)))]));
    }

    // ── filter: string ops ───────────────────────────────────────────────────

    #[test]
    fn test_filter_contains() {
        let r = row(&[("name", json!("Alice Wonderland"))]);
        assert!(apply_filter(&r, &[cond("name", FilterOp::Contains, Some(json!("Alice")))]));
        assert!(!apply_filter(&r, &[cond("name", FilterOp::Contains, Some(json!("Bob")))]));
    }

    #[test]
    fn test_filter_starts_with() {
        let r = row(&[("name", json!("Alice"))]);
        assert!(apply_filter(&r, &[cond("name", FilterOp::StartsWith, Some(json!("Ali")))]));
        assert!(!apply_filter(&r, &[cond("name", FilterOp::StartsWith, Some(json!("ice")))]));
    }

    #[test]
    fn test_filter_ends_with() {
        let r = row(&[("name", json!("Alice"))]);
        assert!(apply_filter(&r, &[cond("name", FilterOp::EndsWith, Some(json!("ice")))]));
        assert!(!apply_filter(&r, &[cond("name", FilterOp::EndsWith, Some(json!("Ali")))]));
    }

    // ── filter: null ops ─────────────────────────────────────────────────────

    #[test]
    fn test_filter_is_null() {
        let r_null = row(&[("deleted_at", json!(null))]);
        let r_val = row(&[("deleted_at", json!("2024-01-01"))]);
        assert!(apply_filter(&r_null, &[cond("deleted_at", FilterOp::IsNull, None)]));
        assert!(!apply_filter(&r_val, &[cond("deleted_at", FilterOp::IsNull, None)]));
    }

    #[test]
    fn test_filter_is_not_null() {
        let r_null = row(&[("deleted_at", json!(null))]);
        let r_val = row(&[("deleted_at", json!("2024-01-01"))]);
        assert!(!apply_filter(&r_null, &[cond("deleted_at", FilterOp::IsNotNull, None)]));
        assert!(apply_filter(&r_val, &[cond("deleted_at", FilterOp::IsNotNull, None)]));
    }

    #[test]
    fn test_filter_missing_field_treated_as_null() {
        let r = row(&[("name", json!("Alice"))]);
        // "missing_col" not in row → treated as Null
        assert!(apply_filter(&r, &[cond("missing_col", FilterOp::IsNull, None)]));
    }

    // ── filter: AND combination ──────────────────────────────────────────────

    #[test]
    fn test_filter_and_all_pass() {
        let r = row(&[("age", json!(25)), ("active", json!(1))]);
        let conditions = vec![
            cond("age", FilterOp::Gte, Some(json!(18))),
            cond("active", FilterOp::Eq, Some(json!(1))),
        ];
        assert!(apply_filter(&r, &conditions));
    }

    #[test]
    fn test_filter_and_one_fails() {
        let r = row(&[("age", json!(15)), ("active", json!(1))]);
        let conditions = vec![
            cond("age", FilterOp::Gte, Some(json!(18))),
            cond("active", FilterOp::Eq, Some(json!(1))),
        ];
        assert!(!apply_filter(&r, &conditions));
    }

    #[test]
    fn test_filter_no_conditions_passes_all() {
        let r = row(&[("age", json!(5))]);
        assert!(apply_filter(&r, &[]));
    }

    // ── projection ───────────────────────────────────────────────────────────

    #[test]
    fn test_projection_keeps_specified_fields() {
        let r = row(&[("id", json!(1)), ("name", json!("Alice")), ("age", json!(30))]);
        let fields: Vec<String> = vec!["id".into(), "name".into()];
        let projected = apply_projection(r, &fields);
        assert_eq!(projected.len(), 2);
        assert!(projected.contains_key("id"));
        assert!(projected.contains_key("name"));
        assert!(!projected.contains_key("age"));
    }

    #[test]
    fn test_projection_empty_keeps_all() {
        let r = row(&[("id", json!(1)), ("name", json!("Alice")), ("age", json!(30))]);
        let projected = apply_projection(r, &[]);
        assert_eq!(projected.len(), 3);
    }

    #[test]
    fn test_projection_unknown_field_is_ignored() {
        let r = row(&[("id", json!(1))]);
        let fields: Vec<String> = vec!["id".into(), "nonexistent".into()];
        let projected = apply_projection(r, &fields);
        assert_eq!(projected.len(), 1);
        assert!(projected.contains_key("id"));
    }
}
