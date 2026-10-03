//! The `forge_accounts` table (docs/architecture.md, Data model). Only
//! `AccountService` calls these, on the core database's worker.

use rusqlite::{params, Connection, OptionalExtension};

use super::accounts::AccountCheck;
use crate::db::{enum_name, parse_enum};
use crate::models::{AppResult, ForgeAccount, ForgeKind};

const COLUMNS: &str = "kind, login, display_name, email, token_kind, expires_at, scopes_json,
    missing_json, read_only, checked_at, user_id";

fn row_to_account(r: &rusqlite::Row<'_>) -> rusqlite::Result<ForgeAccount> {
    let scopes: Option<String> = r.get(6)?;
    let missing: String = r.get(7)?;
    Ok(ForgeAccount {
        kind: parse_enum(r.get(0)?)?,
        login: r.get(1)?,
        display_name: r.get(2)?,
        email: r.get(3)?,
        token_kind: parse_enum(r.get(4)?)?,
        expires_at: r.get(5)?,
        scopes: scopes.and_then(|s| serde_json::from_str(&s).ok()),
        missing: serde_json::from_str(&missing).unwrap_or_default(),
        read_only: r.get(8)?,
        checked_at: r.get(9)?,
        user_id: r.get(10)?,
    })
}

pub fn list(conn: &Connection) -> AppResult<Vec<ForgeAccount>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {COLUMNS} FROM forge_accounts ORDER BY kind"
    ))?;
    let rows = stmt.query_map([], row_to_account)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn get(conn: &Connection, kind: ForgeKind) -> AppResult<Option<ForgeAccount>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM forge_accounts WHERE kind = ?1"),
            [enum_name(kind)?],
            row_to_account,
        )
        .optional()?)
}

/// Insert the provider's account, or replace what the last check found.
pub fn save(
    conn: &Connection,
    kind: ForgeKind,
    check: &AccountCheck,
    email: Option<&str>,
    read_only: bool,
    now: &str,
) -> AppResult<()> {
    let scopes = check
        .scopes
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    conn.execute(
        "INSERT INTO forge_accounts (id, kind, host, login, user_id, display_name, email,
             token_kind, expires_at, scopes_json, missing_json, read_only, checked_at, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)
         ON CONFLICT (kind) DO UPDATE SET
             host = excluded.host, login = excluded.login, user_id = excluded.user_id,
             display_name = excluded.display_name, email = excluded.email,
             token_kind = excluded.token_kind, expires_at = excluded.expires_at,
             scopes_json = excluded.scopes_json, missing_json = excluded.missing_json,
             read_only = excluded.read_only, checked_at = excluded.checked_at",
        params![
            uuid::Uuid::new_v4().to_string(),
            enum_name(kind)?,
            kind.host(),
            check.login,
            check.user_id,
            check.display_name,
            email,
            enum_name(check.token_kind)?,
            check.expires_at,
            scopes,
            serde_json::to_string(&check.missing)?,
            read_only,
            now,
        ],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, kind: ForgeKind) -> AppResult<()> {
    conn.execute(
        "DELETE FROM forge_accounts WHERE kind = ?1",
        [enum_name(kind)?],
    )?;
    Ok(())
}
