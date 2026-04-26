use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// Top-level configuration loaded from a YAML file.
#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    pub source: Source,

    /// Optional in-memory filter conditions (AND-combined).
    #[serde(default)]
    pub filter: Vec<FilterCondition>,

    /// Optional list of field names to keep. Empty = keep all.
    #[serde(default)]
    pub projection: Vec<String>,

    // ── Template fields (in priority order, lowest → highest) ──────────

    /// Lowest priority: inline template string.
    pub template: Option<String>,

    /// Second lowest: path to a single template file.
    pub template_file: Option<String>,

    /// Higher priority: list of template specs.
    #[serde(default)]
    pub templates: Vec<TemplateSpec>,
}

/// SQLite data source.
#[derive(Debug, Deserialize, Serialize)]
pub struct Source {
    pub db_path: String,

    /// If provided, this raw SQL is executed directly.
    pub query: Option<String>,

    /// If `query` is absent, the adapter builds `SELECT … FROM <table>`.
    pub table: Option<String>,
}

/// A single filter condition applied in-memory after the query.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct FilterCondition {
    pub field: String,
    pub op: FilterOp,
    /// The comparison value (JSON scalar or null).
    pub value: Option<Value>,
}

/// Supported filter operators.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    Eq,
    Ne,
    Gt,
    Lt,
    Gte,
    Lte,
    Contains,
    StartsWith,
    EndsWith,
    IsNull,
    IsNotNull,
}

/// One entry in the `templates` list – either a file path or an inline string.
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum TemplateSpec {
    /// A bare string is treated as a file path.
    FilePath(String),
    /// `{ file: "…" }`
    FileMap { file: String },
    /// `{ inline: "…" }`
    Inline { inline: String },
}

// ── Resolution helpers ──────────────────────────────────────────────────────

/// A resolved template ready for rendering.
#[derive(Debug, Clone)]
pub enum ResolvedTemplate {
    /// Template body already loaded into memory.
    Body(String),
    /// Path to a file that will be read at render time (stored for display).
    File { path: String, body: String },
}

impl ResolvedTemplate {
    pub fn body(&self) -> &str {
        match self {
            ResolvedTemplate::Body(s) => s,
            ResolvedTemplate::File { body, .. } => body,
        }
    }
}

impl TemplateSpec {
    /// Read the spec into a `ResolvedTemplate`.
    pub fn resolve(&self, base_dir: &Path) -> Result<ResolvedTemplate> {
        match self {
            TemplateSpec::FilePath(p) | TemplateSpec::FileMap { file: p } => {
                let full = base_dir.join(p);
                let body = fs::read_to_string(&full)
                    .with_context(|| format!("reading template file '{}'", full.display()))?;
                Ok(ResolvedTemplate::File {
                    path: full.display().to_string(),
                    body,
                })
            }
            TemplateSpec::Inline { inline } => Ok(ResolvedTemplate::Body(inline.clone())),
        }
    }
}

// ── Config loading ──────────────────────────────────────────────────────────

impl Config {
    /// Parse a YAML config file.
    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("reading config file '{}'", path.display()))?;
        let cfg: Config = serde_yaml::from_str(&text)
            .with_context(|| format!("parsing config file '{}'", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<()> {
        if self.source.query.is_none() && self.source.table.is_none() {
            bail!("config: source must have either 'query' or 'table'");
        }
        Ok(())
    }

    /// Resolve the effective list of templates given optional CLI overrides.
    ///
    /// Priority (highest first):
    /// 1. `cli_templates` – paths supplied via `--template` on the CLI (resolved against `cli_dir`)
    /// 2. `config.templates`            (resolved against `config_dir`)
    /// 3. `config.template_file`        (resolved against `config_dir`)
    /// 4. `config.template` (inline)
    pub fn resolve_templates(
        &self,
        cli_templates: &[String],
        cli_dir: &Path,
        config_dir: &Path,
    ) -> Result<Vec<ResolvedTemplate>> {
        if !cli_templates.is_empty() {
            return cli_templates
                .iter()
                .map(|p| {
                    let full = if Path::new(p).is_absolute() {
                        PathBuf::from(p)
                    } else {
                        cli_dir.join(p)
                    };
                    let body = fs::read_to_string(&full)
                        .with_context(|| format!("reading template file '{}'", full.display()))?;
                    Ok(ResolvedTemplate::File {
                        path: full.display().to_string(),
                        body,
                    })
                })
                .collect();
        }

        if !self.templates.is_empty() {
            return self
                .templates
                .iter()
                .map(|spec| spec.resolve(config_dir))
                .collect();
        }

        if let Some(ref tf) = self.template_file {
            let full = config_dir.join(tf);
            let body = fs::read_to_string(&full)
                .with_context(|| format!("reading template_file '{}'", full.display()))?;
            return Ok(vec![ResolvedTemplate::File {
                path: full.display().to_string(),
                body,
            }]);
        }

        if let Some(ref t) = self.template {
            return Ok(vec![ResolvedTemplate::Body(t.clone())]);
        }

        bail!(
            "no template defined – supply one via --template, or add 'templates', \
             'template_file', or 'template' to the config"
        );
    }
}
