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

impl FilterCondition {
    /// Parse a CLI `--filter` string of the form `FIELD:OP[:VALUE]`.
    ///
    /// `VALUE` is optional for `is_null` / `is_not_null` and required for all
    /// other operators.  The value is interpreted as a JSON number when possible,
    /// otherwise as a plain string.
    pub fn from_cli_str(s: &str) -> Result<Self> {
        // Split on ':' with a max of 3 parts so that values containing ':'
        // (e.g. timestamps) are kept intact.
        let parts: Vec<&str> = s.splitn(3, ':').collect();
        if parts.len() < 2 {
            bail!("invalid --filter '{s}': expected FIELD:OP[:VALUE]");
        }
        let field = parts[0].to_string();
        let op = FilterOp::parse(parts[1])
            .with_context(|| format!("invalid --filter '{s}'"))?;

        let needs_value = !matches!(op, FilterOp::IsNull | FilterOp::IsNotNull);
        let value = if parts.len() >= 3 {
            let raw = parts[2];
            // Prefer integer, then float, then plain string.
            if let Ok(n) = raw.parse::<i64>() {
                Some(Value::Number(n.into()))
            } else if let Ok(f) = raw.parse::<f64>() {
                let n = serde_json::Number::from_f64(f)
                    .ok_or_else(|| anyhow::anyhow!(
                        "invalid --filter '{s}': value '{raw}' cannot be represented as a number (NaN or infinity)"
                    ))?;
                Some(Value::Number(n))
            } else {
                Some(Value::String(raw.to_string()))
            }
        } else if needs_value {
            bail!(
                "invalid --filter '{s}': operator '{}' requires a value (FIELD:OP:VALUE)",
                parts[1]
            );
        } else {
            None
        };

        Ok(FilterCondition { field, op, value })
    }
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

impl FilterOp {
    /// Parse an operator from its snake_case name (same names as YAML).
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "eq" => Ok(FilterOp::Eq),
            "ne" => Ok(FilterOp::Ne),
            "gt" => Ok(FilterOp::Gt),
            "lt" => Ok(FilterOp::Lt),
            "gte" => Ok(FilterOp::Gte),
            "lte" => Ok(FilterOp::Lte),
            "contains" => Ok(FilterOp::Contains),
            "starts_with" => Ok(FilterOp::StartsWith),
            "ends_with" => Ok(FilterOp::EndsWith),
            "is_null" => Ok(FilterOp::IsNull),
            "is_not_null" => Ok(FilterOp::IsNotNull),
            _ => bail!("unknown filter operator '{s}' (valid: eq ne gt lt gte lte contains starts_with ends_with is_null is_not_null)"),
        }
    }

    /// Return the canonical snake_case name for display.
    pub fn as_str(&self) -> &'static str {
        match self {
            FilterOp::Eq => "eq",
            FilterOp::Ne => "ne",
            FilterOp::Gt => "gt",
            FilterOp::Lt => "lt",
            FilterOp::Gte => "gte",
            FilterOp::Lte => "lte",
            FilterOp::Contains => "contains",
            FilterOp::StartsWith => "starts_with",
            FilterOp::EndsWith => "ends_with",
            FilterOp::IsNull => "is_null",
            FilterOp::IsNotNull => "is_not_null",
        }
    }
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
