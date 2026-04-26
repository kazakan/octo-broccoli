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
