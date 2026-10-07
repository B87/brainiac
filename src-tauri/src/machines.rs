//! Machines (SPEC.md, Remote hosts; docs/architecture.md, Machines): a
//! computer Brainiac reaches over SSH, and the host key the user approved
//! for it. A feature that puts something on a machine is a role on it with
//! its own table; in v0.5 the only role is a run host (`agents::hosts`),
//! which asks this service for the connection instead of keeping one.

pub mod ssh;

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use crate::db::Db;
use crate::models::{now_rfc3339, AppError, AppResult};
use ssh::Target;

/// A saved machine. `approved` is false after a restore, or after its role
/// was removed while the machine still holds that role's work.
#[derive(Debug, Clone)]
pub struct Machine {
    pub id: String,
    pub name: String,
    pub user: String,
    pub host: String,
    pub port: u16,
    pub identity_path: Option<String>,
    /// The host key the user approved, `SHA256:…`.
    pub fingerprint: String,
    pub approved: bool,
}

/// What the user confirmed in Add host or Confirm Host Key.
pub struct Approval {
    pub name: String,
    pub user: String,
    pub host: String,
    pub port: u16,
    pub identity_path: Option<String>,
    /// The fingerprint the preview showed.
    pub fingerprint: String,
    /// Set when the machine is saved with a different fingerprint.
    pub accept_changed_key: bool,
}

pub struct MachineService {
    db: Db,
    data_dir: PathBuf,
}

impl MachineService {
    pub fn new(db: Db, data_dir: &Path) -> Self {
        Self {
            db,
            data_dir: data_dir.to_path_buf(),
        }
    }

    /// The fingerprint of the key the host presents now. Nothing is saved
    /// or trusted.
    pub async fn fingerprint(&self, host: &str, port: u16) -> AppResult<String> {
        ssh::fingerprints(host, port)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| AppError::dependency("The host did not present an SSH key."))
    }

    /// Trust the key the user was shown and save the machine, or confirm a
    /// saved one again. The same user, host, and port is the same machine.
    pub async fn approve(&self, request: Approval) -> AppResult<Machine> {
        let target = ssh::target(
            &request.user,
            &request.host,
            request.port,
            request.identity_path.as_deref(),
            PathBuf::from("/dev/null"),
        )?;
        let lines = ssh::keyscan(&target.host, target.port).await?;
        let prints = lines
            .iter()
            .map(|l| ssh::fingerprint(l))
            .collect::<AppResult<Vec<_>>>()?;
        // Only the key the user was shown. Other types from the same scan
        // stay untrusted, even when they arrived beside the confirmed one.
        let lines = lines_matching(&lines, &prints, &request.fingerprint);
        if lines.is_empty() {
            return Err(AppError::validation(
                "The host key changed since it was shown. Look again before approving it.",
            ));
        }
        let existing = self
            .db
            .call({
                let user = target.user.clone();
                let host = target.host.clone();
                let port = target.port;
                move |conn| find(conn, &user, &host, port)
            })
            .await?;
        if let Some(machine) = &existing {
            if machine.fingerprint != request.fingerprint && !request.accept_changed_key {
                return Err(AppError::validation(
                    "This host's key changed. Approve the new fingerprint to continue.",
                ));
            }
        }
        let id = existing
            .map(|m| m.id)
            .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
        ssh::write_known_hosts(&self.known_hosts(&id), &lines)?;
        let name = match request.name.trim() {
            "" => target.host.clone(),
            trimmed => trimmed.to_string(),
        };
        let machine = Machine {
            id,
            name,
            user: target.user,
            host: target.host,
            port: target.port,
            identity_path: request.identity_path,
            fingerprint: request.fingerprint,
            approved: true,
        };
        self.db
            .call(move |conn| {
                save(conn, &machine, &now_rfc3339())?;
                Ok(machine)
            })
            .await
    }

    pub async fn get(&self, id: &str) -> AppResult<Machine> {
        let id = id.to_string();
        self.db
            .call(move |conn| {
                load(conn, &id)?.ok_or_else(|| AppError::not_found("That host is not saved."))
            })
            .await
    }

    /// Where to connect, pinned to the approved key in this machine's own
    /// `known_hosts`.
    pub fn target(&self, machine: &Machine) -> AppResult<Target> {
        ssh::target(
            &machine.user,
            &machine.host,
            machine.port,
            machine.identity_path.as_deref(),
            self.known_hosts(&machine.id),
        )
    }

    /// Stop trusting the machine but keep it, for a role that still has
    /// work there. Approving it again restores it.
    pub async fn withdraw(&self, id: &str) -> AppResult<()> {
        let id = id.to_string();
        self.db
            .call(move |conn| {
                conn.execute(
                    "UPDATE machines SET approved = 0, version = version + 1, updated_at = ?2
                     WHERE id = ?1",
                    params![id, now_rfc3339()],
                )?;
                Ok(())
            })
            .await
    }

    /// Delete the machine and its approved key. A role still on it refuses
    /// this through its foreign key.
    pub async fn forget(&self, id: &str) -> AppResult<()> {
        let gone = id.to_string();
        self.db
            .call(move |conn| {
                conn.execute("DELETE FROM machines WHERE id = ?1", [&gone])?;
                Ok(())
            })
            .await?;
        let _ = std::fs::remove_dir_all(self.machine_dir(id));
        Ok(())
    }

    fn machine_dir(&self, id: &str) -> PathBuf {
        self.data_dir.join("machines").join(id)
    }

    fn known_hosts(&self, id: &str) -> PathBuf {
        self.machine_dir(id).join("known_hosts")
    }
}

/// The `ssh-keyscan` lines whose fingerprint is the one the user confirmed.
fn lines_matching(lines: &[String], prints: &[String], fingerprint: &str) -> Vec<String> {
    lines
        .iter()
        .zip(prints.iter())
        .filter(|(_, print)| ssh::same_fingerprint(fingerprint, print))
        .map(|(line, _)| line.clone())
        .collect()
}

const COLUMNS: &str =
    "id, name, ssh_user, ssh_host, ssh_port, identity_path, fingerprint, approved";

/// One machine, read in a role's own query so both come from one snapshot.
pub fn load(conn: &Connection, id: &str) -> AppResult<Option<Machine>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM machines WHERE id = ?1"),
            [id],
            row_of,
        )
        .optional()?)
}

fn find(conn: &Connection, user: &str, host: &str, port: u16) -> AppResult<Option<Machine>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {COLUMNS} FROM machines
                 WHERE ssh_user = ?1 AND ssh_host = ?2 AND ssh_port = ?3"
            ),
            params![user, host, port],
            row_of,
        )
        .optional()?)
}

fn row_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<Machine> {
    Ok(Machine {
        id: r.get(0)?,
        name: r.get(1)?,
        user: r.get(2)?,
        host: r.get(3)?,
        port: r.get::<_, i64>(4)? as u16,
        identity_path: r.get(5)?,
        fingerprint: r.get(6)?,
        approved: r.get::<_, i64>(7)? != 0,
    })
}

fn save(conn: &Connection, machine: &Machine, now: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO machines (id, name, ssh_user, ssh_host, ssh_port, identity_path,
            fingerprint, approved, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?8)
         ON CONFLICT(id) DO UPDATE SET name = ?2, ssh_user = ?3, ssh_host = ?4, ssh_port = ?5,
            identity_path = ?6, fingerprint = ?7, approved = 1, version = version + 1,
            updated_at = ?8",
        params![
            machine.id,
            machine.name,
            machine.user,
            machine.host,
            machine.port,
            machine.identity_path,
            machine.fingerprint,
            now
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_keeps_only_the_confirmed_key() {
        let lines = vec!["ed25519-line".into(), "rsa-line".into()];
        let prints = vec!["SHA256:ed".into(), "SHA256:rsa".into()];
        assert_eq!(
            lines_matching(&lines, &prints, "SHA256:ed"),
            vec!["ed25519-line".to_string()]
        );
        assert!(lines_matching(&lines, &prints, "SHA256:other").is_empty());
    }
}
