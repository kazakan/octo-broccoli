mod config;
mod db;
mod pipeline;
mod template;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

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
        } => execute_run(&config_path, db_override.as_deref(), &cli_templates),
    }
}

fn execute_run(
    config_path: &Path,
    db_override: Option<&str>,
    cli_templates: &[String],
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

    // 4. Query SQLite
    let raw_rows = db::query_rows(&cfg.source, db_path_override.as_deref())?;

    // 5. Filter
    let filtered: Vec<_> = raw_rows
        .into_iter()
        .filter(|row| pipeline::apply_filter(row, &cfg.filter))
        .collect();

    // 6. Projection
    let projected: Vec<_> = filtered
        .into_iter()
        .map(|row| pipeline::apply_projection(row, &cfg.projection))
        .collect();

    // 7. Render templates
    template::render_all(&resolved_templates, &projected)?;

    Ok(())
}
