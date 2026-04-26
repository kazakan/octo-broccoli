use anyhow::{Context, Result};
use handlebars::Handlebars;
use serde_json::{json, Map, Value};

use crate::config::ResolvedTemplate;

/// Render all resolved templates and write each output to stdout, separated by
/// a blank line when there are multiple templates.
pub fn render_all(
    templates: &[ResolvedTemplate],
    rows: &[Map<String, Value>],
) -> Result<()> {
    let mut hbs = Handlebars::new();
    // Keep unresolved variables silent (empty string) instead of erroring.
    hbs.set_strict_mode(false);

    let count = rows.len();
    let data = json!({ "rows": rows, "count": count });

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
