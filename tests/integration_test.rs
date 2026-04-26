/// Integration tests for the `ocbro` binary.
///
/// Each test builds a temporary SQLite database and YAML config (with any
/// template files) inside a temp directory, then invokes the compiled binary
/// and asserts on stdout / stderr / exit code.
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn ocbro_bin() -> &'static str {
    env!("CARGO_BIN_EXE_ocbro")
}

/// Self-cleaning temporary directory.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ocbro_it_{}_{}",
            tag,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Populate a `users` table used by all tests.
///
/// | id | name  | age | active | deleted_at |
/// |----|-------|-----|--------|------------|
/// |  1 | Alice |  30 |  1     | NULL       |
/// |  2 | Bob   |  17 |  1     | NULL       |
/// |  3 | Carol |  25 |  0     | 2024-01-01 |
/// |  4 | Dave  |  22 |  1     | NULL       |
fn create_users_db(db_path: &Path) {
    let conn = Connection::open(db_path).unwrap();
    conn.execute_batch(
        "CREATE TABLE users (
             id         INTEGER PRIMARY KEY,
             name       TEXT    NOT NULL,
             age        INTEGER NOT NULL,
             active     INTEGER NOT NULL DEFAULT 1,
             deleted_at TEXT
         );
         INSERT INTO users VALUES (1, 'Alice', 30, 1, NULL);
         INSERT INTO users VALUES (2, 'Bob',   17, 1, NULL);
         INSERT INTO users VALUES (3, 'Carol', 25, 0, '2024-01-01');
         INSERT INTO users VALUES (4, 'Dave',  22, 1, NULL);",
    )
    .unwrap();
}

/// Run `ocbro` with the given arguments, using `dir` as the working directory.
/// Returns `(stdout, stderr, success)`.
fn run(dir: &Path, args: &[&str]) -> (String, String, bool) {
    let out = Command::new(ocbro_bin())
        .current_dir(dir)
        .args(args)
        .output()
        .expect("failed to spawn ocbro");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

// ── Tests ────────────────────────────────────────────────────────────────────

// ── 1. count with an inline template (config `template:`) ────────────────────

#[test]
fn test_count_inline_template() {
    let d = TempDir::new("count_inline");
    create_users_db(&d.file("test.db"));
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  table: users\ntemplate: \"count={{count}}\"\n",
    )
    .unwrap();

    let (stdout, _, ok) = run(d.path(), &["run", "config.yaml"]);
    assert!(ok, "ocbro exited with an error");
    assert_eq!(stdout.trim(), "count=4");
}

// ── 2. Multi-row iteration with {{#each rows}} ────────────────────────────────

#[test]
fn test_each_rows_template() {
    let d = TempDir::new("each_rows");
    create_users_db(&d.file("test.db"));
    // Use \n literal in the inline template.
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  table: users\ntemplate: |\n  {{#each rows}}- {{name}}\n  {{/each}}\n",
    )
    .unwrap();

    let (stdout, _, ok) = run(d.path(), &["run", "config.yaml"]);
    assert!(ok);
    assert!(stdout.contains("- Alice"));
    assert!(stdout.contains("- Bob"));
    assert!(stdout.contains("- Carol"));
    assert!(stdout.contains("- Dave"));
}

// ── 3. Single-row shortcut: {{row.field}} without #each ──────────────────────

#[test]
fn test_single_row_variable() {
    let d = TempDir::new("row_var");
    create_users_db(&d.file("test.db"));
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  table: users\n\
         filter:\n  - field: id\n    op: eq\n    value: 1\n\
         template: \"Name: {{row.name}}, Age: {{row.age}}\"\n",
    )
    .unwrap();

    let (stdout, _, ok) = run(d.path(), &["run", "config.yaml"]);
    assert!(ok);
    assert_eq!(stdout.trim(), "Name: Alice, Age: 30");
}

// ── 4. Config filter: age >= 18 excludes Bob ─────────────────────────────────

#[test]
fn test_config_filter_age_gte() {
    let d = TempDir::new("config_filter");
    create_users_db(&d.file("test.db"));
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  table: users\n\
         filter:\n  - field: age\n    op: gte\n    value: 18\n\
         template: \"count={{count}}\"\n",
    )
    .unwrap();

    let (stdout, _, ok) = run(d.path(), &["run", "config.yaml"]);
    assert!(ok);
    assert_eq!(stdout.trim(), "count=3"); // Alice(30), Carol(25), Dave(22)
}

// ── 5. CLI --filter appends a new condition ───────────────────────────────────

#[test]
fn test_cli_filter_new_condition() {
    let d = TempDir::new("cli_filter_new");
    create_users_db(&d.file("test.db"));
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  table: users\n\
         template: \"count={{count}}\"\n",
    )
    .unwrap();

    // active=1 excludes Carol (active=0), leaving Alice, Bob, Dave.
    let (stdout, _, ok) = run(d.path(), &["run", "config.yaml", "--filter", "active:eq:1"]);
    assert!(ok);
    assert_eq!(stdout.trim(), "count=3");
}

// ── 6. CLI --filter overrides a config filter on the same field ──────────────

#[test]
fn test_cli_filter_overrides_config_filter() {
    let d = TempDir::new("cli_filter_override");
    create_users_db(&d.file("test.db"));
    // Config: age >= 18 → would keep Alice(30), Carol(25), Dave(22)
    // CLI: age < 25  → replaces config, keeps Bob(17) + Dave(22) = 2 rows
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  table: users\n\
         filter:\n  - field: age\n    op: gte\n    value: 18\n\
         template: \"count={{count}}\"\n",
    )
    .unwrap();

    let (stdout, stderr, ok) = run(d.path(), &["run", "config.yaml", "--filter", "age:lt:25"]);
    assert!(ok);
    assert_eq!(stdout.trim(), "count=2"); // Bob(17) and Dave(22)
    // A warning about the override must appear on stderr.
    assert!(stderr.contains("warning"), "expected warning on stderr, got: {stderr}");
    assert!(stderr.contains("age"));
}

// ── 7. Multiple --filter flags are AND-combined ───────────────────────────────

#[test]
fn test_multiple_cli_filters_and() {
    let d = TempDir::new("multi_filter");
    create_users_db(&d.file("test.db"));
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  table: users\n\
         template: \"count={{count}}\"\n",
    )
    .unwrap();

    // active=1 AND age>=18 → Alice(30), Dave(22)  (Bob underage; Carol inactive)
    let (stdout, _, ok) = run(
        d.path(),
        &["run", "config.yaml", "--filter", "active:eq:1", "--filter", "age:gte:18"],
    );
    assert!(ok);
    assert_eq!(stdout.trim(), "count=2");
}

// ── 8. is_null filter ────────────────────────────────────────────────────────

#[test]
fn test_filter_is_null() {
    let d = TempDir::new("is_null");
    create_users_db(&d.file("test.db"));
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  table: users\n\
         template: \"count={{count}}\"\n",
    )
    .unwrap();

    let (stdout, _, ok) = run(
        d.path(),
        &["run", "config.yaml", "--filter", "deleted_at:is_null"],
    );
    assert!(ok);
    assert_eq!(stdout.trim(), "count=3"); // Alice, Bob, Dave
}

// ── 9. is_not_null filter ────────────────────────────────────────────────────

#[test]
fn test_filter_is_not_null() {
    let d = TempDir::new("is_not_null");
    create_users_db(&d.file("test.db"));
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  table: users\n\
         template: \"count={{count}}\"\n",
    )
    .unwrap();

    let (stdout, _, ok) = run(
        d.path(),
        &["run", "config.yaml", "--filter", "deleted_at:is_not_null"],
    );
    assert!(ok);
    assert_eq!(stdout.trim(), "count=1"); // Carol
}

// ── 10. contains filter ───────────────────────────────────────────────────────

#[test]
fn test_filter_contains() {
    let d = TempDir::new("contains");
    create_users_db(&d.file("test.db"));
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  table: users\n\
         template: \"count={{count}}\"\n",
    )
    .unwrap();

    // "Al" is in "Alice" only.
    let (stdout, _, ok) = run(
        d.path(),
        &["run", "config.yaml", "--filter", "name:contains:Al"],
    );
    assert!(ok);
    assert_eq!(stdout.trim(), "count=1");
}

// ── 11. starts_with filter ────────────────────────────────────────────────────

#[test]
fn test_filter_starts_with() {
    let d = TempDir::new("starts_with");
    create_users_db(&d.file("test.db"));
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  table: users\n\
         template: \"count={{count}}\"\n",
    )
    .unwrap();

    // "B" starts only "Bob".
    let (stdout, _, ok) = run(
        d.path(),
        &["run", "config.yaml", "--filter", "name:starts_with:B"],
    );
    assert!(ok);
    assert_eq!(stdout.trim(), "count=1");
}

// ── 12. ends_with filter ─────────────────────────────────────────────────────

#[test]
fn test_filter_ends_with() {
    let d = TempDir::new("ends_with");
    create_users_db(&d.file("test.db"));
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  table: users\n\
         template: \"count={{count}}\"\n",
    )
    .unwrap();

    // "e" ends "Alice" and "Dave".
    let (stdout, _, ok) = run(
        d.path(),
        &["run", "config.yaml", "--filter", "name:ends_with:e"],
    );
    assert!(ok);
    assert_eq!(stdout.trim(), "count=2");
}

// ── 13. Projection limits visible fields ──────────────────────────────────────

#[test]
fn test_projection_removes_fields() {
    let d = TempDir::new("projection");
    create_users_db(&d.file("test.db"));
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  table: users\n\
         filter:\n  - field: id\n    op: eq\n    value: 1\n\
         projection: [id, name]\n\
         template: \"{{row.id}} {{row.name}} age={{row.age}}\"\n",
    )
    .unwrap();

    let (stdout, _, ok) = run(d.path(), &["run", "config.yaml"]);
    assert!(ok);
    // age is projected out → Handlebars renders it as empty string
    let out = stdout.trim();
    assert!(out.contains("1"));
    assert!(out.contains("Alice"));
    assert!(out.contains("age="), "output was: {out}");
    assert!(!out.contains("30"), "age should be absent; output was: {out}");
}

// ── 14. Raw SQL query via `query:` ────────────────────────────────────────────

#[test]
fn test_raw_sql_query() {
    let d = TempDir::new("raw_sql");
    create_users_db(&d.file("test.db"));
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  query: \"SELECT * FROM users WHERE age >= 18\"\n\
         template: \"count={{count}}\"\n",
    )
    .unwrap();

    let (stdout, _, ok) = run(d.path(), &["run", "config.yaml"]);
    assert!(ok);
    assert_eq!(stdout.trim(), "count=3"); // Alice, Carol, Dave
}

// ── 15. --db flag overrides config db_path ────────────────────────────────────

#[test]
fn test_db_override() {
    let d = TempDir::new("db_override");
    create_users_db(&d.file("real.db"));
    // Create an empty DB (same schema, no rows).
    let conn = Connection::open(d.file("empty.db")).unwrap();
    conn.execute_batch(
        "CREATE TABLE users (id INTEGER, name TEXT, age INTEGER, active INTEGER, deleted_at TEXT);",
    )
    .unwrap();

    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./real.db\n  table: users\n\
         template: \"count={{count}}\"\n",
    )
    .unwrap();

    let (stdout, _, ok) = run(d.path(), &["run", "config.yaml", "--db", "./empty.db"]);
    assert!(ok);
    assert_eq!(stdout.trim(), "count=0");
}

// ── 16. --template flag overrides all config templates ───────────────────────

#[test]
fn test_cli_template_overrides_config() {
    let d = TempDir::new("tpl_override");
    create_users_db(&d.file("test.db"));
    fs::write(d.file("report.tpl"), "total={{count}}").unwrap();
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  table: users\n\
         template: \"SHOULD NOT APPEAR\"\n",
    )
    .unwrap();

    let (stdout, _, ok) = run(d.path(), &["run", "config.yaml", "--template", "report.tpl"]);
    assert!(ok);
    assert_eq!(stdout.trim(), "total=4");
    assert!(!stdout.contains("SHOULD NOT APPEAR"));
}

// ── 17. config `template_file:` ───────────────────────────────────────────────

#[test]
fn test_config_template_file() {
    let d = TempDir::new("tpl_file");
    create_users_db(&d.file("test.db"));
    fs::write(d.file("report.tpl"), "rows={{count}}").unwrap();
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  table: users\n\
         template_file: ./report.tpl\n",
    )
    .unwrap();

    let (stdout, _, ok) = run(d.path(), &["run", "config.yaml"]);
    assert!(ok);
    assert_eq!(stdout.trim(), "rows=4");
}

// ── 18. config `templates:` list renders multiple templates ──────────────────

#[test]
fn test_config_templates_list() {
    let d = TempDir::new("tpl_list");
    create_users_db(&d.file("test.db"));
    fs::write(d.file("detail.tpl"), "detail={{count}}").unwrap();
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  table: users\n\
         templates:\n\
           - inline: \"header={{count}}\"\n\
           - file: ./detail.tpl\n",
    )
    .unwrap();

    let (stdout, _, ok) = run(d.path(), &["run", "config.yaml"]);
    assert!(ok);
    assert!(stdout.contains("header=4"));
    assert!(stdout.contains("detail=4"));
}

// ── 19. Multiple --template CLI flags render all templates ────────────────────

#[test]
fn test_multiple_cli_template_flags() {
    let d = TempDir::new("multi_tpl");
    create_users_db(&d.file("test.db"));
    fs::write(d.file("a.tpl"), "A={{count}}").unwrap();
    fs::write(d.file("b.tpl"), "B={{count}}").unwrap();
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  table: users\n\
         template: \"IGNORED\"\n",
    )
    .unwrap();

    let (stdout, _, ok) = run(
        d.path(),
        &["run", "config.yaml", "--template", "a.tpl", "--template", "b.tpl"],
    );
    assert!(ok);
    assert!(stdout.contains("A=4"));
    assert!(stdout.contains("B=4"));
}

// ── 20. Empty result set: count=0, row renders as empty string ───────────────

#[test]
fn test_empty_result_set() {
    let d = TempDir::new("empty_result");
    create_users_db(&d.file("test.db"));
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  table: users\n\
         filter:\n  - field: age\n    op: gt\n    value: 999\n\
         template: \"count={{count}}\"\n",
    )
    .unwrap();

    let (stdout, _, ok) = run(d.path(), &["run", "config.yaml"]);
    assert!(ok);
    assert_eq!(stdout.trim(), "count=0");
}

// ── 21. {{row}} is first row when multiple rows are present ──────────────────

#[test]
fn test_row_variable_is_first_row() {
    let d = TempDir::new("row_first");
    create_users_db(&d.file("test.db"));
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  query: \"SELECT * FROM users ORDER BY id\"\n\
         template: \"first={{row.name}}\"\n",
    )
    .unwrap();

    let (stdout, _, ok) = run(d.path(), &["run", "config.yaml"]);
    assert!(ok);
    assert_eq!(stdout.trim(), "first=Alice");
}

// ── 22. Numeric range: gt + lte combined ─────────────────────────────────────

#[test]
fn test_numeric_range_filter() {
    let d = TempDir::new("num_range");
    create_users_db(&d.file("test.db"));
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  table: users\n\
         template: \"count={{count}}\"\n",
    )
    .unwrap();

    // 20 < age <= 25 → Carol(25), Dave(22) → 2 rows
    let (stdout, _, ok) = run(
        d.path(),
        &["run", "config.yaml", "--filter", "age:gt:20", "--filter", "age:lte:25"],
    );
    assert!(ok);
    assert_eq!(stdout.trim(), "count=2");
}

// ── 23. ne (not-equal) filter ────────────────────────────────────────────────

#[test]
fn test_filter_ne() {
    let d = TempDir::new("ne");
    create_users_db(&d.file("test.db"));
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  table: users\n\
         template: \"count={{count}}\"\n",
    )
    .unwrap();

    // Exclude Alice (id=1) → Bob, Carol, Dave
    let (stdout, _, ok) = run(d.path(), &["run", "config.yaml", "--filter", "id:ne:1"]);
    assert!(ok);
    assert_eq!(stdout.trim(), "count=3");
}

// ── 24. Error: config file does not exist ────────────────────────────────────

#[test]
fn test_error_missing_config_file() {
    let d = TempDir::new("missing_cfg");
    let (_, stderr, ok) = run(d.path(), &["run", "nonexistent.yaml"]);
    assert!(!ok, "ocbro should have exited with a non-zero status");
    assert!(!stderr.is_empty(), "should have printed an error message");
}

// ── 25. Error: config has no template defined ────────────────────────────────

#[test]
fn test_error_no_template() {
    let d = TempDir::new("no_tpl");
    create_users_db(&d.file("test.db"));
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  table: users\n",
    )
    .unwrap();

    let (_, stderr, ok) = run(d.path(), &["run", "config.yaml"]);
    assert!(!ok);
    assert!(stderr.contains("template"));
}

// ── 26. Error: invalid --filter format ───────────────────────────────────────

#[test]
fn test_error_invalid_filter_format() {
    let d = TempDir::new("bad_filter");
    create_users_db(&d.file("test.db"));
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  table: users\n\
         template: \"count={{count}}\"\n",
    )
    .unwrap();

    let (_, stderr, ok) = run(d.path(), &["run", "config.yaml", "--filter", "badformat"]);
    assert!(!ok);
    assert!(!stderr.is_empty());
}

// ── 27. Template priority: CLI --template > config templates[] ───────────────

#[test]
fn test_template_priority_cli_wins() {
    let d = TempDir::new("tpl_priority");
    create_users_db(&d.file("test.db"));
    fs::write(d.file("cli.tpl"), "CLI={{count}}").unwrap();
    fs::write(
        d.file("config.yaml"),
        "source:\n  db_path: ./test.db\n  table: users\n\
         templates:\n  - inline: \"CONFIG={{count}}\"\n",
    )
    .unwrap();

    let (stdout, _, ok) = run(d.path(), &["run", "config.yaml", "--template", "cli.tpl"]);
    assert!(ok);
    assert!(stdout.contains("CLI=4"));
    assert!(!stdout.contains("CONFIG="));
}
