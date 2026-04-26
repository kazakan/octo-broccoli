mod config;
mod db;
mod pipeline;
mod template;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use config::FilterCondition;

// ── CLI definition ───────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "ocbro",
    version,
    about = "Fast, read-only SQLite → Filter → Projection → Template CLI"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Execute the pipeline defined in a YAML config file.
    Run {
        /// Path to the YAML configuration file.
        config: PathBuf,

        /// Override source.db_path from the config.
        #[arg(long, value_name = "PATH")]
        db: Option<String>,

        /// Template file(s) to use (overrides all template config).
        /// Can be specified multiple times.
        #[arg(long = "template", value_name = "FILE", action = clap::ArgAction::Append)]
        templates: Vec<String>,

        /// Add or override a filter condition.  Format: FIELD:OP[:VALUE].
        /// Can be specified multiple times.  If a FIELD already has a filter
        /// in the config file the CLI value replaces it (a warning is emitted).
        ///
        /// Examples:
        ///   --filter age:gte:18
        ///   --filter name:contains:Alice
        ///   --filter deleted_at:is_null
        #[arg(long = "filter", value_name = "FIELD:OP[:VALUE]", action = clap::ArgAction::Append)]
        filters: Vec<String>,
    },
}

// ── Entry point ──────────────────────────────────────────────────────────────

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Run {
            config: config_path,
            db: db_override,
            templates: cli_templates,
            filters: cli_filters,
        } => execute_run(&config_path, db_override.as_deref(), &cli_templates, &cli_filters),
    }
}

fn execute_run(
    config_path: &Path,
    db_override: Option<&str>,
    cli_templates: &[String],
    cli_filter_strs: &[String],
) -> Result<()> {
    // 1. Load config
    let cfg = config::Config::load(config_path)?;

    // The base directory for relative paths (templates, db) is the directory
    // that contains the config file.
    let config_dir = config_path
        .parent()
        .unwrap_or_else(|| Path::new("."));

    // 2. Resolve the effective db path (relative paths are anchored to cwd).
    let db_path_override = db_override
        .map(|p| -> Result<String> {
            if Path::new(p).is_absolute() {
                Ok(p.to_string())
            } else {
                let cwd = std::env::current_dir().context("determining current directory")?;
                Ok(cwd.join(p).display().to_string())
            }
        })
        .transpose()?;

    // 3. Resolve templates (CLI templates are anchored to cwd; config templates to config_dir).
    let cwd = std::env::current_dir().context("determining current directory")?;
    let resolved_templates = cfg.resolve_templates(cli_templates, &cwd, config_dir)?;

    // 4. Merge CLI filters with config filters.
    let effective_filters = merge_filters(cfg.filter.clone(), cli_filter_strs)?;

    // 5. Query SQLite
    let raw_rows = db::query_rows(&cfg.source, db_path_override.as_deref())?;

    // 6. Filter
    let filtered: Vec<_> = raw_rows
        .into_iter()
        .filter(|row| pipeline::apply_filter(row, &effective_filters))
        .collect();

    // 7. Projection
    let projected: Vec<_> = filtered
        .into_iter()
        .map(|row| pipeline::apply_projection(row, &cfg.projection))
        .collect();

    // 8. Render templates
    template::render_all(&resolved_templates, &projected)?;

    Ok(())
}

// ── Filter merging ───────────────────────────────────────────────────────────

/// Merge CLI `--filter` strings into the config's filter list.
///
/// If a CLI filter targets the same field as an existing config filter, the
/// config filter is replaced and a warning is printed to stderr.
/// New CLI filters (unknown fields) are appended.
fn merge_filters(
    config_filters: Vec<FilterCondition>,
    cli_filter_strs: &[String],
) -> Result<Vec<FilterCondition>> {
    if cli_filter_strs.is_empty() {
        return Ok(config_filters);
    }

    let cli_filters: Vec<FilterCondition> = cli_filter_strs
        .iter()
        .map(|s| FilterCondition::from_cli_str(s))
        .collect::<Result<_>>()?;

    // Keep config filters that are not superseded by a CLI filter.
    let mut result: Vec<FilterCondition> = config_filters
        .into_iter()
        .filter_map(|cf| {
            if let Some(cli_f) = cli_filters.iter().find(|f| f.field == cf.field) {
                eprintln!(
                    "warning: --filter overrides config filter for field '{}' \
                    (config: {}; cli: {})",
                    cf.field,
                    format_filter_display(&cf),
                    format_filter_display(cli_f),
                );
                None // Replaced by the CLI filter below.
            } else {
                Some(cf)
            }
        })
        .collect();

    // Append all CLI filters (both overrides and new additions).
    result.extend(cli_filters);

    Ok(result)
}

fn format_filter_display(f: &FilterCondition) -> String {
    match &f.value {
        Some(v) => format!("{}:{}", f.op.as_str(), v),
        None => f.op.as_str().to_string(),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use config::{FilterCondition, FilterOp};
    use serde_json::json;

    fn make_cond(field: &str, op: FilterOp, value: Option<serde_json::Value>) -> FilterCondition {
        FilterCondition { field: field.to_string(), op, value }
    }

    #[test]
    fn test_merge_filters_no_cli_filters() {
        let config_filters = vec![make_cond("age", FilterOp::Gte, Some(json!(18)))];
        let result = merge_filters(config_filters.clone(), &[]).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].field, "age");
    }

    #[test]
    fn test_merge_filters_new_cli_filter_appended() {
        let config_filters = vec![make_cond("age", FilterOp::Gte, Some(json!(18)))];
        let result = merge_filters(config_filters, &["active:eq:1".to_string()]).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.iter().any(|f| f.field == "age"));
        assert!(result.iter().any(|f| f.field == "active"));
    }

    #[test]
    fn test_merge_filters_cli_overrides_same_field() {
        let config_filters = vec![make_cond("age", FilterOp::Gte, Some(json!(18)))];
        // CLI supplies a different op+value for the same field.
        let result = merge_filters(config_filters, &["age:lt:30".to_string()]).unwrap();
        // Only one filter for 'age' should remain (the CLI one).
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].field, "age");
        assert_eq!(result[0].op, FilterOp::Lt);
        assert_eq!(result[0].value, Some(json!(30)));
    }

    #[test]
    fn test_merge_filters_multiple_cli_filters() {
        let config_filters = vec![];
        let cli = vec!["age:gte:18".to_string(), "deleted_at:is_null".to_string()];
        let result = merge_filters(config_filters, &cli).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_merge_filters_invalid_cli_filter_returns_error() {
        let result = merge_filters(vec![], &["badformat".to_string()]);
        assert!(result.is_err());
    }
}
