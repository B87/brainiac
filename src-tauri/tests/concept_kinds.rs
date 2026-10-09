//! Migration 0012: a known concept's kind `system` becomes `tool`
//! (docs/design/code-explanations.md, Concept classification). The database
//! is built as 0.6.0 leaves it, by the test, so no binary fixture is checked in.

use brainiac_lib::db;
use rusqlite::{params, Connection};

/// A database at migration 0011, with known concepts of every kind.
fn as_0_6_0(path: &std::path::Path) {
    let mut conn = Connection::open(path).unwrap();
    conn.pragma_update(None, "application_id", db::APPLICATION_ID)
        .unwrap();
    let eleven = db::Store {
        migrations: &db::CORE.migrations[..11],
        ..db::CORE
    };
    db::migrate(&mut conn, &eleven).unwrap();
    let add = |id: &str, kind: &str, name: &str, repository: &str, merged: Option<&str>| {
        conn.execute(
            "INSERT INTO known_concepts (id, kind, name, key, repository_id, merged_into,
                learned_at, description)
             VALUES (?1, ?2, ?3, lower(?3), ?4, ?5, '2026-10-09T00:00:00Z', 'words')",
            params![id, kind, name, repository, merged],
        )
        .unwrap();
    };
    add("embed", "language", "go:embed", "", None);
    add("git", "system", "git", "", None);
    // A merge group of one kind: "Git" merged into "git".
    add("git2", "system", "Git CLI", "", Some("git"));
    add("dkim", "system", "DKIM", "", None);
    add("rule", "project_pattern", "Limit rule", "repo-a", None);
}

#[test]
fn system_concepts_become_tools_and_everything_else_is_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join(db::CORE_FILE);
    as_0_6_0(&path);

    let mut conn = Connection::open(&path).unwrap();
    db::migrate(&mut conn, &db::CORE).unwrap();

    let mut stmt = conn
        .prepare("SELECT id, kind, repository_id, merged_into, description FROM known_concepts ORDER BY id")
        .unwrap();
    let rows: Vec<(String, String, String, Option<String>, String)> = stmt
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let kinds: Vec<(&str, &str)> = rows.iter().map(|r| (r.0.as_str(), r.1.as_str())).collect();
    assert_eq!(
        kinds,
        [
            ("dkim", "tool"),
            ("embed", "language"),
            ("git", "tool"),
            ("git2", "tool"),
            ("rule", "project_pattern"),
        ]
    );
    // The merge group, the repository of a project pattern, and the words are kept.
    let git2 = rows.iter().find(|r| r.0 == "git2").unwrap();
    assert_eq!(git2.3.as_deref(), Some("git"));
    assert_eq!(rows.iter().find(|r| r.0 == "rule").unwrap().2, "repo-a");
    assert!(rows.iter().all(|r| r.4 == "words"));

    // The new kinds are allowed and the old one is not.
    let insert = |kind: &str| {
        conn.execute(
            "INSERT INTO known_concepts (id, kind, name, key, learned_at)
             VALUES (?1, ?2, 'x', ?1, '2026-10-09T00:00:00Z')",
            params![format!("new-{kind}"), kind],
        )
    };
    assert!(insert("protocol").is_ok());
    assert!(insert("technique").is_ok());
    assert!(insert("tool_two").is_err());
    assert!(insert("system").is_err());

    // A merge still points at its concept: forgetting it unmerges nothing else.
    conn.execute("DELETE FROM known_concepts WHERE id = 'git'", [])
        .unwrap();
    let after: Option<String> = conn
        .query_row(
            "SELECT merged_into FROM known_concepts WHERE id = 'git2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(after, None, "ON DELETE SET NULL still applies");
}

#[test]
fn a_stored_explanation_reads_its_old_kind_as_a_tool() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join(db::HISTORY_FILE);
    let mut conn = Connection::open(&path).unwrap();
    conn.pragma_update(None, "application_id", db::HISTORY.application_id)
        .unwrap();
    let six = db::Store {
        migrations: &db::HISTORY.migrations[..6],
        ..db::HISTORY
    };
    db::migrate(&mut conn, &six).unwrap();
    // As 0.6.0 stored it: compact JSON, a concept of the kind `system`, and a
    // note whose text mentions it in words (its quotes are escaped).
    let stored = r#"{"concepts":[{"name":"git","kind":"system"},{"name":"go","kind":"language"}],"notes":[{"text":"says \"kind\":\"system\""}]}"#;
    conn.execute(
        "INSERT INTO explanations (id, repository_id, repository_name, subject_kind, subject_ref,
            title, base, tip, profile_id, agent, provider, payment, host_id, host_name, model,
            depth, questions, time_limit_minutes, state, explanation, created_at)
         VALUES ('e', 'r', 'repo', 'commit', 'abc', 't', 'b', 'abc', 'p', 'claude_code',
            'anthropic', 'api_key', 'local', 'This Mac', '', 'brief', 1, 5, 'ready', ?1,
            '2026-10-09T00:00:00Z')",
        [stored],
    )
    .unwrap();
    db::migrate(&mut conn, &db::HISTORY).unwrap();
    let after: String = conn
        .query_row(
            "SELECT explanation FROM explanations WHERE id = 'e'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(after.contains(r#"{"name":"git","kind":"tool"}"#), "{after}");
    assert!(after.contains(r#""kind":"language""#));
    // The note's own words are not touched.
    assert!(after.contains(r#"says \"kind\":\"system\""#), "{after}");
}
