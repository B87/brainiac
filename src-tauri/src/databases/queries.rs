//! Saved queries (SPEC.md, Databases: Saved queries): named SQL in the core
//! database, with a folder, a description, the connection it runs on, and
//! the last values of its parameters. A save carries the version it was
//! made from, so a query changed elsewhere is not overwritten.

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::db::Db;
use crate::models::{
    now_rfc3339, AppError, AppResult, ErrorCode, ParamValue, SaveQueryRequest, SavedQuery,
};

pub struct SavedQueryService {
    db: Db,
}

const COLUMNS: &str =
    "id, name, folder, description, connection_id, sql, parameters_json, version, updated_at";

fn from_row(r: &Row<'_>) -> rusqlite::Result<SavedQuery> {
    let parameters: String = r.get(6)?;
    Ok(SavedQuery {
        id: r.get(0)?,
        name: r.get(1)?,
        folder: r.get(2)?,
        description: r.get(3)?,
        connection_id: r.get(4)?,
        sql: r.get(5)?,
        parameters: serde_json::from_str(&parameters).unwrap_or_default(),
        version: r.get(7)?,
        updated_at: r.get(8)?,
    })
}

fn get(conn: &Connection, id: &str) -> AppResult<Option<SavedQuery>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM saved_queries WHERE id = ?1"),
            [id],
            from_row,
        )
        .optional()?)
}

/// A folder as `Billing/Monthly`: no empty parts, no slashes at the ends.
fn normalize_folder(folder: &str) -> String {
    folder
        .split('/')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

fn conflict() -> AppError {
    AppError::new(
        ErrorCode::Conflict,
        "This saved query changed since the tab opened it. The tab keeps its text; save it as a new query or reopen the saved one.",
    )
}

impl SavedQueryService {
    pub fn new(db: Db) -> Self {
        SavedQueryService { db }
    }

    pub async fn list(&self) -> AppResult<Vec<SavedQuery>> {
        self.db
            .call(|conn| {
                let mut statement = conn.prepare(&format!(
                    "SELECT {COLUMNS} FROM saved_queries ORDER BY folder COLLATE NOCASE, name COLLATE NOCASE"
                ))?;
                let rows = statement.query_map([], from_row)?;
                Ok(rows.collect::<Result<_, _>>()?)
            })
            .await
    }

    pub async fn get(&self, id: &str) -> AppResult<SavedQuery> {
        let id = id.to_string();
        self.db
            .call(move |conn| get(conn, &id))
            .await?
            .ok_or_else(|| AppError::not_found("That saved query no longer exists."))
    }

    /// Save Query: create one, or update it if it is still at the version
    /// the tab opened.
    pub async fn save(&self, request: SaveQueryRequest) -> AppResult<SavedQuery> {
        let name = request.name.trim().to_string();
        if name.is_empty() {
            return Err(AppError::validation("Name the query."));
        }
        if name.chars().count() > 200 {
            return Err(AppError::validation(
                "A query's name is at most 200 characters.",
            ));
        }
        if request.sql.trim().is_empty() {
            return Err(AppError::validation("There is no SQL to save."));
        }
        let folder = normalize_folder(&request.folder);
        let description = request.description.trim().to_string();
        let now = now_rfc3339();
        self.db
            .call(move |conn| {
                let tx = conn.transaction()?;
                if let Some(connection) = &request.connection_id {
                    let exists: bool = tx.query_row(
                        "SELECT EXISTS (SELECT 1 FROM db_connections WHERE id = ?1)",
                        [connection],
                        |r| r.get(0),
                    )?;
                    if !exists {
                        return Err(AppError::not_found("That connection no longer exists."));
                    }
                }
                let id = match &request.id {
                    Some(id) => {
                        let version: Option<i64> = tx
                            .query_row("SELECT version FROM saved_queries WHERE id = ?1", [id], |r| r.get(0))
                            .optional()?;
                        match version {
                            None => return Err(AppError::not_found("That saved query no longer exists.")),
                            Some(v) if request.expected_version.is_some_and(|e| e != v) => {
                                return Err(conflict())
                            }
                            Some(_) => {}
                        }
                        tx.execute(
                            "UPDATE saved_queries SET name = ?2, folder = ?3, description = ?4,
                                connection_id = ?5, sql = ?6, version = version + 1, updated_at = ?7
                              WHERE id = ?1",
                            params![id, name, folder, description, request.connection_id, request.sql, now],
                        )?;
                        id.clone()
                    }
                    None => {
                        let id = uuid::Uuid::new_v4().to_string();
                        tx.execute(
                            "INSERT INTO saved_queries (id, name, folder, description, connection_id, sql,
                                version, created_at, updated_at)
                              VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?7)",
                            params![id, name, folder, description, request.connection_id, request.sql, now],
                        )?;
                        id
                    }
                };
                let saved = get(&tx, &id)?.ok_or_else(|| AppError::db("The query was not saved."))?;
                tx.commit()?;
                Ok(saved)
            })
            .await
    }

    pub async fn delete(&self, id: &str, expected_version: i64) -> AppResult<()> {
        let id = id.to_string();
        self.db
            .call(move |conn| {
                let version: Option<i64> = conn
                    .query_row(
                        "SELECT version FROM saved_queries WHERE id = ?1",
                        [&id],
                        |r| r.get(0),
                    )
                    .optional()?;
                match version {
                    None => Ok(()),
                    Some(v) if v != expected_version => Err(conflict()),
                    Some(_) => {
                        conn.execute("DELETE FROM saved_queries WHERE id = ?1", [&id])?;
                        Ok(())
                    }
                }
            })
            .await
    }

    /// Keep the values a query last ran with. This does not change its
    /// version: the values are not part of the query the tab edits.
    pub async fn remember_parameters(
        &self,
        id: &str,
        values: Vec<ParamValue>,
    ) -> AppResult<SavedQuery> {
        let id = id.to_string();
        let json = serde_json::to_string(&values)?;
        self.db
            .call(move |conn| {
                conn.execute(
                    "UPDATE saved_queries SET parameters_json = ?2 WHERE id = ?1",
                    params![id, json],
                )?;
                get(conn, &id)?
                    .ok_or_else(|| AppError::not_found("That saved query no longer exists."))
            })
            .await
    }
}
