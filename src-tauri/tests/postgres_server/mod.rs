//! A throwaway PostgreSQL server for the database tests: `initdb` and
//! `pg_ctl` from `PATH` in a temporary folder, on a free port, with password
//! authentication and, when asked, TLS with a certificate authority made for
//! the test. Tests skip with a message when PostgreSQL is not installed.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use brainiac_lib::credentials::Secret;
use brainiac_lib::databases::postgres::PgTarget;
use brainiac_lib::models::DbTls;

pub const USER: &str = "postgres";
pub const PASSWORD: &str = "test password";

pub struct PgServer {
    pub dir: tempfile::TempDir,
    pub port: u16,
    data: PathBuf,
    /// The CA that signed the server's certificate, when TLS is on.
    pub ca_file: Option<PathBuf>,
}

fn installed(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn run(command: &mut Command) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{command:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

impl PgServer {
    /// A running server, or `None` (after saying why) when PostgreSQL is not installed.
    pub fn start() -> Option<PgServer> {
        Self::start_with(false, "")
    }

    /// A running server with `pg_stat_statements` loaded, or `None` when
    /// this PostgreSQL does not ship it.
    pub fn start_with_statements() -> Option<PgServer> {
        Self::start_with(false, " -c shared_preload_libraries=pg_stat_statements")
    }

    /// A running server with TLS on, or `None` without PostgreSQL or `openssl`.
    pub fn start_tls() -> Option<PgServer> {
        if !installed("openssl") {
            eprintln!("skipped: openssl is not installed");
            return None;
        }
        Self::start_with(true, "")
    }

    fn start_with(tls: bool, extra: &str) -> Option<PgServer> {
        if !installed("initdb") || !installed("pg_ctl") {
            eprintln!("skipped: PostgreSQL (initdb, pg_ctl) is not on PATH");
            return None;
        }
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let pwfile = dir.path().join("pw");
        fs::write(&pwfile, PASSWORD).unwrap();
        run(Command::new("initdb")
            .arg("-D")
            .arg(&data)
            .args([
                "-U",
                USER,
                "-A",
                "scram-sha-256",
                "-E",
                "UTF8",
                "--locale=C",
                "--no-sync",
            ])
            .arg(format!("--pwfile={}", pwfile.display())));
        let ca_file = tls.then(|| make_certificates(dir.path(), &data));
        // A free port can be taken by a parallel test before pg_ctl binds
        // it, so a failed start is tried again on another port.
        let mut attempts = 0;
        let port = loop {
            attempts += 1;
            let port = free_port();
            let mut options = format!(
                "-p {port} -c listen_addresses=127.0.0.1 -c unix_socket_directories='' -c fsync=off"
            );
            if tls {
                options
                    .push_str(" -c ssl=on -c ssl_cert_file=server.crt -c ssl_key_file=server.key");
            }
            options.push_str(extra);
            let started = Command::new("pg_ctl")
                .arg("-D")
                .arg(&data)
                .arg("-l")
                .arg(dir.path().join("log"))
                .args(["-w", "-t", "30", "-o", &options, "start"])
                .output()
                .unwrap();
            if started.status.success() {
                break port;
            }
            if attempts < 3 {
                continue;
            }
            assert!(
                !extra.is_empty(),
                "pg_ctl start failed: {}\n{}",
                String::from_utf8_lossy(&started.stderr),
                fs::read_to_string(dir.path().join("log")).unwrap_or_default()
            );
            eprintln!("skipped: PostgreSQL did not start with{extra}");
            return None;
        };
        Some(PgServer {
            dir,
            port,
            data,
            ca_file,
        })
    }

    pub fn target(&self) -> PgTarget {
        PgTarget {
            host: "127.0.0.1".into(),
            port: self.port,
            database: "postgres".into(),
            user: USER.into(),
            password: Some(Secret::new(PASSWORD).unwrap()),
            tls: DbTls::Off,
            ca_file: None,
            statement_timeout: Duration::from_secs(30),
            application_name: "Brainiac".into(),
        }
    }

    /// Run SQL as the superuser with `psql`, for setting up fixtures.
    pub fn psql(&self, sql: &str) {
        run(Command::new("psql")
            .env("PGPASSWORD", PASSWORD)
            .args([
                "-h",
                "127.0.0.1",
                "-U",
                USER,
                "-d",
                "postgres",
                "-v",
                "ON_ERROR_STOP=1",
                "-q",
            ])
            .arg("-p")
            .arg(self.port.to_string())
            .arg("-c")
            .arg(sql));
    }
}

impl Drop for PgServer {
    fn drop(&mut self) {
        let _ = Command::new("pg_ctl")
            .arg("-D")
            .arg(&self.data)
            .args(["-m", "immediate", "-w", "stop"])
            .output();
    }
}

/// A CA and a server certificate for `localhost` signed by it, written into
/// the data folder as PostgreSQL expects. Returns the CA's file.
fn make_certificates(dir: &Path, data: &Path) -> PathBuf {
    let ca_key = dir.join("ca.key");
    let ca_crt = dir.join("ca.crt");
    run(Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "2",
            "-subj",
            "/CN=Brainiac Test CA",
        ])
        .arg("-keyout")
        .arg(&ca_key)
        .arg("-out")
        .arg(&ca_crt));
    let key = data.join("server.key");
    let csr = dir.join("server.csr");
    run(Command::new("openssl")
        .args([
            "req",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-subj",
            "/CN=localhost",
        ])
        .arg("-keyout")
        .arg(&key)
        .arg("-out")
        .arg(&csr));
    let ext = dir.join("ext.cnf");
    fs::write(
        &ext,
        "subjectAltName=DNS:localhost\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n",
    )
    .unwrap();
    run(Command::new("openssl")
        .args(["x509", "-req", "-days", "2", "-CAcreateserial"])
        .arg("-in")
        .arg(&csr)
        .arg("-CA")
        .arg(&ca_crt)
        .arg("-CAkey")
        .arg(&ca_key)
        .arg("-extfile")
        .arg(&ext)
        .arg("-out")
        .arg(data.join("server.crt")));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
    }
    ca_crt
}
