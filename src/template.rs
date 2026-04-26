use anyhow::{Context, Result};
use handlebars::Handlebars;
use serde_json::{json, Map, Value};

use crate::config::ResolvedTemplate;

/// Render all resolved templates to a single String.
///
/// The template context contains:
/// - `rows`  – the full array of result rows
/// - `count` – number of rows (`rows.length`)
/// - `row`   – the **first** row as a flat object (or `null` if there are no
///   rows).  Allows single-row templates to omit `{{#each rows}}`.
pub fn render_to_string(
    templates: &[ResolvedTemplate],
    rows: &[Map<String, Value>],
) -> Result<String> {
    let mut hbs = Handlebars::new();
    // Keep unresolved variables silent (empty string) instead of erroring.
    hbs.set_strict_mode(false);

    let count = rows.len();
    // Inject the first row directly so templates can use {{row.field}} without
    // wrapping everything in {{#each rows}}.
    let first_row: Value = rows
        .first()
        .cloned()
        .map(Value::Object)
        .unwrap_or(Value::Null);
    let data = json!({ "rows": rows, "count": count, "row": first_row });

    let mut out = String::new();
    for (i, tpl) in templates.iter().enumerate() {
        let output = hbs
            .render_template(tpl.body(), &data)
            .with_context(|| {
                let label = match tpl {
                    ResolvedTemplate::File { path, .. } => format!("template file '{path}'"),
                    ResolvedTemplate::Body(_) => "inline template".to_string(),
                };
                format!("rendering {label}")
            })?;

        if i > 0 {
            out.push('\n');
        }
        out.push_str(&output);
    }
    out.push('\n');
    Ok(out)
}

/// Render all resolved templates and write each output to stdout.
pub fn render_all(
    templates: &[ResolvedTemplate],
    rows: &[Map<String, Value>],
) -> Result<()> {
    let output = render_to_string(templates, rows)?;
    print!("{output}");
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ResolvedTemplate;
    use serde_json::json;

    fn inline(s: &str) -> ResolvedTemplate {
        ResolvedTemplate::Body(s.to_string())
    }

    fn make_row(pairs: &[(&str, Value)]) -> Map<String, Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn test_render_count() {
        let rows = vec![
            make_row(&[("id", json!(1))]),
            make_row(&[("id", json!(2))]),
        ];
        let out = render_to_string(&[inline("Total: {{count}}")], &rows).unwrap();
        assert_eq!(out.trim(), "Total: 2");
    }

    #[test]
    fn test_render_each_rows() {
        let rows = vec![
            make_row(&[("name", json!("Alice")), ("age", json!(30))]),
            make_row(&[("name", json!("Bob")), ("age", json!(25))]),
        ];
        let tpl = "{{#each rows}}- {{name}} ({{age}})\n{{/each}}";
        let out = render_to_string(&[inline(tpl)], &rows).unwrap();
        assert!(out.contains("- Alice (30)"));
        assert!(out.contains("- Bob (25)"));
    }

    #[test]
    fn test_render_single_row_via_row_variable() {
        let rows = vec![make_row(&[("name", json!("Alice")), ("age", json!(30))])];
        let tpl = "Name: {{row.name}}\nAge: {{row.age}}";
        let out = render_to_string(&[inline(tpl)], &rows).unwrap();
        assert!(out.contains("Name: Alice"));
        assert!(out.contains("Age: 30"));
    }

    #[test]
    fn test_render_row_is_null_when_no_rows() {
        let rows: Vec<Map<String, Value>> = vec![];
        // Handlebars with strict_mode=false renders missing/null as empty string.
        let tpl = "count={{count}}";
        let out = render_to_string(&[inline(tpl)], &rows).unwrap();
        assert_eq!(out.trim(), "count=0");
    }

    #[test]
    fn test_render_multiple_templates_separated_by_blank_line() {
        let rows = vec![make_row(&[("name", json!("Alice"))])];
        let templates = vec![inline("A: {{row.name}}"), inline("B: {{row.name}}")];
        let out = render_to_string(&templates, &rows).unwrap();
        // The two outputs are separated by a blank line (the '\n' pushed between them)
        // then the final '\n' appended at the end.
        assert!(out.contains("A: Alice"));
        assert!(out.contains("B: Alice"));
        // There must be a blank line separating the two outputs.
        assert!(out.contains("A: Alice\nB: Alice"));
    }

    #[test]
    fn test_render_row_is_first_when_multiple_rows() {
        let rows = vec![
            make_row(&[("name", json!("Alice"))]),
            make_row(&[("name", json!("Bob"))]),
        ];
        let tpl = "first={{row.name}}";
        let out = render_to_string(&[inline(tpl)], &rows).unwrap();
        assert_eq!(out.trim(), "first=Alice");
    }
}
