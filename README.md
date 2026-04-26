# ocbro – Simple template filling CLI

A simple, **read-only** CLI tool that:

1. Reads rows from a **SQLite** database
2. Applies **in-memory filters** (AND conditions)
3. Applies a **projection** (select only certain fields)
4. Renders one or more **Handlebars templates**

---

## Build

```bash
cargo build --release
# binary: ./target/release/ocbro
```

---

## Usage

```
ocbro run <CONFIG> [OPTIONS]

Arguments:
  <CONFIG>               Path to the YAML config file

Options:
  --db <PATH>            Override source.db_path from the config
  --template <FILE>      Template file(s) – can be repeated; overrides config templates
  --filter FIELD:OP[:VALUE]
                         Add or override a filter condition (can be repeated).
                         If the field already has a filter in the config the CLI
                         value replaces it and a warning is printed to stderr.
  -h, --help             Print help
```

### Examples

```bash
# Basic run
ocbro run config.yaml

# Override the database path
ocbro run config.yaml --db ./override.db

# Single template override
ocbro run config.yaml --template report.tpl

# Multiple template overrides
ocbro run config.yaml \
  --template summary.tpl \
  --template detail.tpl

# Add a filter from the CLI (combined AND with config filters)
ocbro run config.yaml --filter age:gte:18

# Multiple CLI filters
ocbro run config.yaml --filter active:eq:1 --filter age:gte:18

# Filter that needs no value
ocbro run config.yaml --filter deleted_at:is_null

# Override a config filter (prints a warning to stderr)
ocbro run config.yaml --filter age:lt:30
```

---

## Config file format (YAML)

```yaml
source:
  db_path: ./data.db
  table: users            # OR use 'query' for raw SQL
  # query: "SELECT * FROM users WHERE active = 1"

filter:                   # optional; all conditions are AND-combined
  - field: active
    op: eq
    value: 1
  - field: age
    op: gte
    value: 18

projection:               # optional; empty = keep all fields
  - id
  - name
  - age

# ── Template options (highest → lowest priority) ──────────────────────
# 1. CLI --template (overrides everything)
# 2. 'templates' list (multiple entries rendered in order)
# 3. 'template_file' (single file path)
# 4. 'template' (inline string)

templates:
  - inline: "Total: {{count}}"
  - file: ./templates/detail.tpl
  - ./templates/summary.tpl      # bare string = file path

template_file: ./report.tpl      # fallback if 'templates' is absent

template: |                      # lowest-priority fallback
  {{#each rows}}{{name}}
  {{/each}}
```

### Filter operators

| `op`          | Description                          |
|---------------|--------------------------------------|
| `eq`          | equal                                |
| `ne`          | not equal                            |
| `gt`          | greater than (numeric)               |
| `lt`          | less than (numeric)                  |
| `gte`         | greater than or equal (numeric)      |
| `lte`         | less than or equal (numeric)         |
| `contains`    | string contains substring            |
| `starts_with` | string starts with prefix            |
| `ends_with`   | string ends with suffix              |
| `is_null`     | field is NULL                        |
| `is_not_null` | field is not NULL                    |

---

## Template context

Templates receive a single JSON object:

```json
{
  "rows":  [ { "id": 1, "name": "Alice", ... }, ... ],
  "count": 3,
  "row":   { "id": 1, "name": "Alice", ... }
}
```

| Variable | Type | Description |
|----------|------|-------------|
| `rows`   | array  | All result rows after filter + projection |
| `count`  | number | Number of rows (`rows.length`) |
| `row`    | object \| null | First row, or `null` when there are no results. Lets single-row templates skip `{{#each rows}}` |

Use [Handlebars](https://handlebarsjs.com/) syntax:

```handlebars
{{! multi-row: iterate with #each }}
Total: {{count}}
{{#each rows}}- {{name}} ({{age}})
{{/each}}
```

```handlebars
{{! single-row: use {{row.field}} directly }}
Name: {{row.name}}
Age:  {{row.age}}
```

---

## Project structure

```
src/
  main.rs        – CLI entry point (clap)
  config.rs      – YAML config structs + template resolution
  db.rs          – SQLite adapter (rusqlite)
  pipeline.rs    – filter + projection
  template.rs    – Handlebars rendering
examples/
  config.yaml
  templates/detail.tpl
```
