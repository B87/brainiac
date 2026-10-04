//! Databases (v0.4, SPEC.md section 11): saved connections to SQLite files
//! and PostgreSQL servers, and a session per query tab that runs statements
//! read only (docs/architecture.md, Databases — v0.4).

pub mod connections;
pub mod driver;
pub mod export;
pub mod health;
pub mod history;
pub mod postgres;
pub mod queries;
pub mod sessions;
pub mod sqlite;
pub mod statements;
pub mod values;

pub use connections::ConnectionService;
pub use queries::SavedQueryService;
pub use sessions::QuerySessions;
