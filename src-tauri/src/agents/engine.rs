//! This Mac's Docker engines, found by their sockets (SPEC.md, Settings →
//! Agents: where runs execute). The default socket belongs to whichever
//! engine claimed it last, so an engine is chosen by its own socket and
//! asked what it is; only engines runs were tested on are offered.
//!
//! Requests go over the engine's Unix socket with `reqwest`, as Health's do
//! (docs/architecture.md, Agent runs — v0.5, Docker).

use std::collections::HashSet;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::models::{AgentEngine, AppError, AppResult};

/// How long a probe waits for an engine to answer.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Engines runs were tested on (docs/design/agent-runs.md, Spike record:
/// local engines).
const SUPPORTED: &[&str] = &["OrbStack", "Docker Desktop"];

/// Where engines put their sockets, tried in this order. The system socket
/// comes last: it is a link to one of the others.
pub fn candidate_sockets() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    vec![
        home.join(".orbstack/run/docker.sock"),
        home.join(".docker/run/docker.sock"),
        home.join(".colima/default/docker.sock"),
        home.join(".rd/docker.sock"),
        PathBuf::from("/var/run/docker.sock"),
    ]
}

/// A client for one engine's API over its socket.
pub fn client(socket: &Path, timeout: Duration) -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .unix_socket(socket)
        .timeout(timeout)
        .build()
        .map_err(|e| AppError::dependency("Docker cannot be reached.").with_details(e.to_string()))
}

/// Every engine socket on this Mac, each once, and the chosen one even when
/// it is gone, each with what its engine said.
pub async fn list(chosen: Option<&str>) -> Vec<AgentEngine> {
    let mut seen = HashSet::new();
    let mut sockets = Vec::new();
    // The chosen socket comes first, so a link to a socket listed after it
    // is shown under the path the user chose.
    let chosen = chosen.map(PathBuf::from);
    for path in chosen.iter().cloned().chain(candidate_sockets()) {
        let is_socket = std::fs::metadata(&path).is_ok_and(|m| m.file_type().is_socket());
        let is_chosen = chosen.as_ref() == Some(&path);
        // A link to a socket already listed is the same engine.
        let real = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if (is_socket || is_chosen) && seen.insert(real) {
            sockets.push(path);
        }
    }
    let probes = sockets.iter().map(|s| probe(s));
    futures_util::future::join_all(probes).await
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Info {
    operating_system: Option<String>,
    server_version: Option<String>,
    #[serde(rename = "NCPU")]
    ncpu: Option<u32>,
    mem_total: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Version {
    api_version: Option<String>,
}

/// What the engine behind a socket is, and whether runs can use it.
pub async fn probe(socket: &Path) -> AgentEngine {
    let mut engine = AgentEngine {
        socket: socket.display().to_string(),
        name: guess_name(socket),
        reachable: false,
        supported: false,
        problem: None,
        server_version: None,
        api_version: None,
        cpus: None,
        memory_bytes: None,
    };
    let answered = async {
        let client = client(socket, PROBE_TIMEOUT)?;
        let info: Info = get(&client, "http://docker/info").await?;
        let version: Version = get(&client, "http://docker/version").await?;
        Ok::<_, AppError>((info, version))
    }
    .await;
    match answered {
        Ok((info, version)) => {
            engine.reachable = true;
            let os = info.operating_system.unwrap_or_default();
            if let Some(known) = SUPPORTED.iter().find(|n| os.contains(*n)) {
                engine.name = known.to_string();
                engine.supported = true;
            } else {
                if !os.is_empty() {
                    engine.name = os;
                }
                engine.problem = Some(
                    "Runs are tested on OrbStack and Docker Desktop; this engine is not offered yet."
                        .to_string(),
                );
            }
            engine.server_version = info.server_version;
            engine.api_version = version.api_version;
            engine.cpus = info.ncpu;
            engine.memory_bytes = info.mem_total;
        }
        Err(e) => engine.problem = Some(e.message),
    }
    engine
}

async fn get<T: serde::de::DeserializeOwned>(client: &reqwest::Client, url: &str) -> AppResult<T> {
    let response = client.get(url).send().await.map_err(|e| {
        AppError::dependency("The engine is not answering. Is it running?")
            .with_details(e.to_string())
    })?;
    if !response.status().is_success() {
        return Err(AppError::dependency(format!(
            "The engine answered {}.",
            response.status()
        )));
    }
    response.json().await.map_err(|e| {
        AppError::dependency("The engine's answer could not be read.").with_details(e.to_string())
    })
}

/// A name from where the socket is, until the engine says what it is.
fn guess_name(socket: &Path) -> String {
    let text = socket.display().to_string();
    if text.contains("/.orbstack/") {
        "OrbStack"
    } else if text.contains("/.docker/") {
        "Docker Desktop"
    } else if text.contains("/.colima/") {
        "Colima"
    } else if text.contains("/.rd/") {
        "Rancher Desktop"
    } else {
        "Docker"
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_missing_socket_is_listed_when_chosen_and_explained() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("docker.sock");
        let engines = list(Some(gone.to_str().unwrap())).await;
        let engine = engines
            .iter()
            .find(|e| e.socket == gone.display().to_string())
            .expect("the chosen socket is listed");
        assert!(!engine.reachable && !engine.supported);
        assert!(engine.problem.is_some());
    }

    #[test]
    fn names_come_from_the_socket_until_the_engine_answers() {
        assert_eq!(
            guess_name(Path::new("/Users/a/.orbstack/run/docker.sock")),
            "OrbStack"
        );
        assert_eq!(
            guess_name(Path::new("/Users/a/.docker/run/docker.sock")),
            "Docker Desktop"
        );
        assert_eq!(guess_name(Path::new("/var/run/docker.sock")), "Docker");
    }
}
