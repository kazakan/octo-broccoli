use anyhow::{Context, Result};
use handlebars::Handlebars;
use serde_json::{json, Map, Value};

use crate::config::ResolvedTemplate;

/// Render all resolved templates and write each output to stdout, separated by
/// a blank line when there are multiple templates.
///
/// The template context contains:
/// - `rows`  – the full array of result rows
/// - `count` – number of rows (`rows.length`)
/// - `row`   – the **first** row as a flat object (or `null` if there are no
///   rows).  Allows single-row templates to omit `{{#each rows}}`.
pub fn render_all(
    templates: &[ResolvedTemplate],
    rows: &[Map<String, Value>],
) -> Result<()> {
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
            println!();
        }
        print!("{output}");
    }

    // Ensure the final output ends with a newline.
    println!();

    Ok(())
}
