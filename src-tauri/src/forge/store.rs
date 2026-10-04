//! The `forge_accounts` table (docs/architecture.md, Data model). Only
//! `AccountService` calls these, on the core database's worker.

use rusqlite::{params, Connection, OptionalExtension};

use super::accounts::AccountCheck;
use crate::db::{enum_name, parse_enum};
use crate::models::{
    AppError, AppResult, CredentialPending, CredentialState, ErrorCode, ForgeAccount, ForgeKind,
    SecretSource,
};

const COLUMNS: &str = "kind, login, display_name, email, token_kind, expires_at, scopes_json,
    missing_json, read_only, checked_at, user_id, secret_source, credential_revision,
    source_approved, credential_pending";

fn row_to_account(r: &rusqlite::Row<'_>) -> rusqlite::Result<ForgeAccount> {
    let scopes: Option<String> = r.get(6)?;
    let missing: String = r.get(7)?;
    let source: String = r.get(11)?;
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
        token_source: serde_json::from_str(&source).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(11, rusqlite::types::Type::Text, Box::new(e))
        })?,
        credential: CredentialState {
            revision: r.get(12)?,
            needs_approval: !r.get::<_, bool>(13)?,
            pending: r
                .get::<_, Option<String>>(14)?
                .map(parse_enum)
                .transpose()?,
        },
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

/// What a save writes besides the check: where the token comes from, the
/// binding's revision, and what is still pending.
pub struct Saved<'a> {
    pub check: &'a AccountCheck,
    pub email: Option<&'a str>,
    pub read_only: bool,
    pub source: &'a SecretSource,
    pub revision: i64,
    pub pending: Option<CredentialPending>,
    pub now: &'a str,
}

/// Insert the provider's account, or replace what the last check found and
/// its binding. A save approves the source it saves.
pub fn save(conn: &Connection, kind: ForgeKind, s: &Saved<'_>) -> AppResult<()> {
    let scopes = s
        .check
        .scopes
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    conn.execute(
        "INSERT INTO forge_accounts (id, kind, host, login, user_id, display_name, email,
             token_kind, expires_at, scopes_json, missing_json, read_only, checked_at, created_at,
             secret_source, credential_revision, source_approved, credential_pending)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13, ?14, ?15, 1, ?16)
         ON CONFLICT (kind) DO UPDATE SET
             host = excluded.host, login = excluded.login, user_id = excluded.user_id,
             display_name = excluded.display_name, email = excluded.email,
             token_kind = excluded.token_kind, expires_at = excluded.expires_at,
             scopes_json = excluded.scopes_json, missing_json = excluded.missing_json,
             read_only = excluded.read_only, checked_at = excluded.checked_at,
             secret_source = excluded.secret_source,
             credential_revision = excluded.credential_revision, source_approved = 1,
             credential_pending = excluded.credential_pending",
        params![
            uuid::Uuid::new_v4().to_string(),
            enum_name(kind)?,
            kind.host(),
            s.check.login,
            s.check.user_id,
            s.check.display_name,
            s.email,
            enum_name(s.check.token_kind)?,
            s.check.expires_at,
            scopes,
            serde_json::to_string(&s.check.missing)?,
            s.read_only,
            s.now,
            serde_json::to_string(s.source)?,
            s.revision,
            s.pending.map(enum_name).transpose()?,
        ],
    )?;
    Ok(())
}

/// Mark a change under way on an existing account.
pub fn mark_pending(
    conn: &Connection,
    kind: ForgeKind,
    pending: CredentialPending,
) -> AppResult<()> {
    conn.execute(
        "UPDATE forge_accounts SET credential_pending = ?2 WHERE kind = ?1",
        params![enum_name(kind)?, enum_name(pending)?],
    )?;
    Ok(())
}

pub fn clear_pending(
    conn: &Connection,
    kind: ForgeKind,
    which: CredentialPending,
) -> AppResult<()> {
    conn.execute(
        "UPDATE forge_accounts SET credential_pending = NULL WHERE kind = ?1 AND credential_pending = ?2",
        params![enum_name(kind)?, enum_name(which)?],
    )?;
    Ok(())
}

/// Confirm a restored source, for the revision the user was shown.
pub fn approve(conn: &Connection, kind: ForgeKind, revision: i64) -> AppResult<()> {
    let changed = conn.execute(
        "UPDATE forge_accounts SET source_approved = 1 WHERE kind = ?1 AND credential_revision = ?2",
        params![enum_name(kind)?, revision],
    )?;
    if changed == 0 {
        return Err(AppError::new(
            ErrorCode::Conflict,
            "This account's token source changed since it was shown. Look at it again.",
        ));
    }
    Ok(())
}

/// What a check of a freshly read token found, for the revision it was
/// read for: kind, expiry, and scopes are replaced, but what the token was
/// found to lack is kept, so a background check never makes a read-only
/// account writable.
pub fn refresh_check(
    conn: &Connection,
    kind: ForgeKind,
    check: &AccountCheck,
    revision: i64,
    now: &str,
) -> AppResult<()> {
    let Some(account) = get(conn, kind)? else {
        return Ok(());
    };
    if account.credential.revision != revision {
        return Ok(());
    }
    let mut missing = account.missing;
    for m in &check.missing {
        if !missing.contains(m) {
            missing.push(m.clone());
        }
    }
    let scopes = check
        .scopes
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    conn.execute(
        "UPDATE forge_accounts SET token_kind = ?2, expires_at = ?3, scopes_json = ?4,
             missing_json = ?5, read_only = read_only OR ?6, checked_at = ?7
          WHERE kind = ?1 AND credential_revision = ?8",
        params![
            enum_name(kind)?,
            enum_name(check.token_kind)?,
            check.expires_at,
            scopes,
            serde_json::to_string(&missing)?,
            !check.missing.is_empty(),
            now,
            revision,
        ],
    )?;
    Ok(())
}

/// Record a permission the provider refused an action for, so that action
/// is off until the token is replaced (SPEC.md, Accounts).
pub fn add_missing(conn: &Connection, kind: ForgeKind, permission: &str) -> AppResult<()> {
    let Some(account) = get(conn, kind)? else {
        return Ok(());
    };
    if account.missing.iter().any(|m| m == permission) {
        return Ok(());
    }
    let mut missing = account.missing;
    missing.push(permission.to_string());
    conn.execute(
        "UPDATE forge_accounts SET missing_json = ?2 WHERE kind = ?1",
        params![enum_name(kind)?, serde_json::to_string(&missing)?],
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
