//! Checks against Docker on this Mac. The controller is a separate process.
//! Quitting the client leaves it running. Engine sockets come from `docker
//! context` at runtime and are not written into the repository.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

use crate::messages::{Request, Response};
use crate::state::PORT;

const PERM_TOOL: &str = "Use the Bash tool to run this command, wait until it finishes, and do nothing else: i=1; while [ $i -le 3 ]; do echo tick-$i >> /workspace/held.txt; i=$((i+1)); sleep 1; done";

struct Engine {
    label: &'static str,
    context: &'static str,
    host: String,
}

struct Client {
    read: BufReader<OwnedReadHalf>,
    write: OwnedWriteHalf,
}

impl Client {
    async fn connect(token: &str) -> anyhow::Result<Self> {
        let stream = TcpStream::connect(("127.0.0.1", PORT)).await?;
        let _ = stream.set_nodelay(true);
        let (read, mut write) = stream.into_split();
        let token_json = serde_json::to_string(token)?;
        write
            .write_all(format!("{{\"token\":{token_json}}}\n").as_bytes())
            .await?;
        Ok(Self {
            read: BufReader::new(read),
            write,
        })
    }

    async fn call(&mut self, request: &Request) -> anyhow::Result<Response> {
        let mut bytes = serde_json::to_vec(request)?;
        bytes.push(b'\n');
        self.write.write_all(&bytes).await?;
        let mut line = String::new();
        let n = self.read.read_line(&mut line).await?;
        if n == 0 {
            anyhow::bail!("controller closed the connection");
        }
        Ok(serde_json::from_str(&line)?)
    }
}

pub async fn run() -> anyhow::Result<()> {
    if std::env::args().nth(3).as_deref() == Some("claude") {
        return claude_only().await;
    }
    let engines = discover()?;
    let mut problems = Vec::new();
    if !engines.iter().any(|engine| engine.context == "orbstack") {
        problems.push("OrbStack is not available".into());
    }
    if !engines
        .iter()
        .any(|engine| engine.context == "desktop-linux")
    {
        problems.push("Docker Desktop is not available".into());
    }
    for engine in &engines {
        if let Err(err) = engine_checks(engine).await {
            problems.push(format!("{}: {err}", engine.label));
        }
    }
    match std::env::var("SPIKE_CLAUDE_TOKEN") {
        Ok(token) if crate::acp::valid_token(&token) => {
            let Some(engine) = engines
                .iter()
                .find(|engine| engine.context == "desktop-linux")
            else {
                problems.push("Claude permission check needs Docker Desktop".into());
                report(&problems)?;
                return Ok(());
            };
            if let Err(err) = claude_permission(engine, &token).await {
                problems.push(format!("Claude permission: {err}"));
            }
        }
        Ok(_) => problems.push("SPIKE_CLAUDE_TOKEN is not a single printable line".into()),
        Err(_) => problems.push("SPIKE_CLAUDE_TOKEN is not set".into()),
    }
    report(&problems)?;
    Ok(())
}

/// Lid-close on this Mac. A tick container on each engine shows whether the
/// VM kept running. The controller session shows the process survived.
pub async fn lid() -> anyhow::Result<()> {
    let engines = discover()?;
    if engines.is_empty() {
        anyhow::bail!("no local engine");
    }
    for engine in &engines {
        let _ = Command::new("docker")
            .env("DOCKER_HOST", &engine.host)
            .args(["pull", "alpine"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = docker(&engine.host, &["rm", "-f", "brainiac-spike-lid-tick"]);
        let status = Command::new("docker")
            .env("DOCKER_HOST", &engine.host)
            .args([
                "run",
                "-d",
                "--name",
                "brainiac-spike-lid-tick",
                "--network",
                "none",
                "alpine",
                "sh",
                "-c",
                "i=0; while true; do i=$((i+1)); echo tick-$i; sleep 1; done",
            ])
            .status()?;
        if !status.success() {
            for started in &engines {
                let _ = docker(&started.host, &["rm", "-f", "brainiac-spike-lid-tick"]);
            }
            anyhow::bail!("{} tick container did not start", engine.label);
        }
    }
    let engine = &engines[0];
    ensure_image(&engine.host, "brainiac-spike-stub:local", "images/stub")?;
    let dir = state_dir("lid");
    let _ = fs::remove_dir_all(&dir);
    let token = write_token(&dir)?;
    let child = spawn_controller(&engine.host, &dir)?;
    let outcome = async {
        wait_until_listening().await?;
        let mut client = Client::connect(&token).await?;
        let started = client
            .call(&Request::Start {
                deadline_secs: 3600,
            })
            .await?;
        let Response::Started { run_id, .. } = started else {
            anyhow::bail!("stub did not start: {started:?}");
        };
        let Response::Status { cursor, .. } = client.call(&Request::Status).await? else {
            anyhow::bail!("status failed before sleep");
        };
        let before: Vec<_> = engines
            .iter()
            .map(|engine| {
                (
                    engine.label,
                    engine.host.clone(),
                    tick_count(&engine.host).unwrap_or(0),
                )
            })
            .collect();
        println!(
            "SLEEP NOW. Session is up on {}. Close the lid for about 20 seconds. If an external display keeps the Mac awake, this waits until the machine actually suspends.",
            engine.label
        );
        let _ = std::io::stdout().flush();
        let wall = SystemTime::now();
        let mono = Instant::now();
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let wall_elapsed = wall.elapsed().unwrap_or_default();
            let mono_elapsed = mono.elapsed();
            if wall_elapsed > mono_elapsed + Duration::from_secs(15) {
                let asleep = wall_elapsed.saturating_sub(mono_elapsed).as_secs();
                let awake = mono_elapsed.as_secs();
                println!("wake after about {asleep}s asleep, {awake}s awake");
                let mut client = Client::connect(&token).await?;
                match client.call(&Request::Status).await? {
                    Response::Status {
                        run_id: Some(id),
                        running,
                        ..
                    } if id == run_id && running => {}
                    other => anyhow::bail!("session did not survive lid-close: {other:?}"),
                }
                let beats = texts_after(&mut client, cursor)
                    .await?
                    .into_iter()
                    .filter(|text| text.starts_with("beat:"))
                    .count() as u64;
                println!(
                    "{} controller journaled {beats} beats, {awake}s awake, {asleep}s asleep",
                    engine.label
                );
                // Both clocks run only while the Mac is awake, so a VM that
                // slept writes about `awake` lines, not `awake + asleep`.
                if beats > awake + 5 {
                    anyhow::bail!("controller kept journaling through the suspend");
                }
                for (label, host, earlier) in before {
                    let added = tick_count(&host).unwrap_or(0).saturating_sub(earlier);
                    println!("{label}: {added} ticks, {awake}s awake, {asleep}s asleep");
                    if added > awake + 5 {
                        anyhow::bail!("{label} kept writing through the suspend");
                    }
                }
                println!("PASS lid-close");
                return Ok(());
            }
        }
    }
    .await;
    shutdown(child);
    for engine in &engines {
        let _ = docker(&engine.host, &["rm", "-f", "brainiac-spike-lid-tick"]);
        cleanup(&engine.host);
    }
    outcome
}

async fn claude_only() -> anyhow::Result<()> {
    let engines = discover()?;
    let Some(engine) = engines
        .iter()
        .find(|engine| engine.context == "desktop-linux")
    else {
        anyhow::bail!("Claude permission check needs Docker Desktop");
    };
    let token = std::env::var("SPIKE_CLAUDE_TOKEN")
        .map_err(|_| anyhow::anyhow!("SPIKE_CLAUDE_TOKEN is not set"))?;
    if !crate::acp::valid_token(&token) {
        anyhow::bail!("SPIKE_CLAUDE_TOKEN is not a single printable line");
    }
    claude_permission(engine, &token).await
}

fn report(problems: &[String]) -> anyhow::Result<()> {
    if problems.is_empty() {
        return Ok(());
    }
    anyhow::bail!("{}", problems.join("; "))
}

fn discover() -> anyhow::Result<Vec<Engine>> {
    let specs = [
        ("Docker Desktop", "desktop-linux"),
        ("OrbStack", "orbstack"),
    ];
    let mut engines = Vec::new();
    for (label, context) in specs {
        let Some(host) = context_host(context) else {
            println!("{label}: context {context} is not available");
            continue;
        };
        engines.push(Engine {
            label,
            context,
            host,
        });
    }
    Ok(engines)
}

fn context_host(context: &str) -> Option<String> {
    let output = Command::new("docker")
        .args([
            "context",
            "inspect",
            context,
            "--format",
            "{{.Endpoints.docker.Host}}",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let host = String::from_utf8(output.stdout).ok()?;
    let host = host.trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

async fn engine_checks(engine: &Engine) -> anyhow::Result<()> {
    println!("--- {}", engine.label);
    ensure_image(&engine.host, "brainiac-spike-stub:local", "images/stub")?;
    quota(engine)?;
    app_quit(engine).await?;
    Ok(())
}

fn quota(engine: &Engine) -> anyhow::Result<()> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/probe-vm-quota.sh");
    let output = Command::new("bash")
        .arg(&script)
        .env("DOCKER_HOST", &engine.host)
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    print!("{stdout}");
    if !stderr.is_empty() {
        eprint!("{stderr}");
    }
    if !output.status.success() || !stdout.lines().any(|line| line == "PASS vm-quota") {
        anyhow::bail!("{} workspace limit was not enforced", engine.label);
    }
    println!("PASS workspace {}", engine.label);
    Ok(())
}

async fn app_quit(engine: &Engine) -> anyhow::Result<()> {
    cleanup(&engine.host);
    let dir = state_dir(&format!("quit-{}", engine.context));
    let _ = fs::remove_dir_all(&dir);
    let token = write_token(&dir)?;
    let child = spawn_controller(&engine.host, &dir)?;
    let outcome = async {
        wait_until_listening().await?;
        let mut client = Client::connect(&token).await?;
        let Response::Started { run_id, cursor, .. } =
            client.call(&Request::Start { deadline_secs: 120 }).await?
        else {
            anyhow::bail!("stub did not start");
        };
        drop(client);
        tokio::time::sleep(Duration::from_secs(3)).await;
        let mut client = Client::connect(&token).await?;
        match client.call(&Request::Status).await? {
            Response::Status {
                run_id: Some(id),
                running: true,
                ..
            } if id == run_id => {}
            other => {
                anyhow::bail!("session was not still running after the client left: {other:?}")
            }
        }
        let beats = texts_after(&mut client, cursor)
            .await?
            .into_iter()
            .filter(|text| text.starts_with("beat:"))
            .count();
        if beats < 2 {
            anyhow::bail!("expected the session to keep beating, saw {beats}");
        }
        println!(
            "PASS app quit {}: same session, {beats} beats after the client left ({})",
            engine.label,
            engine_banner(&dir)
        );
        Ok(())
    }
    .await;
    let err = controller_log(&dir);
    shutdown(child);
    cleanup(&engine.host);
    outcome.map_err(|err_value| {
        if err.is_empty() {
            err_value
        } else {
            anyhow::anyhow!("{err_value}\n{err}")
        }
    })
}

async fn claude_permission(engine: &Engine, token: &str) -> anyhow::Result<()> {
    ensure_image(&engine.host, "brainiac-spike-claude:local", "images/claude")?;
    cleanup(&engine.host);
    let dir = state_dir("claude-permission");
    let _ = fs::remove_dir_all(&dir);
    let host_token = write_token(&dir)?;
    let child = spawn_controller(&engine.host, &dir)?;
    let outcome = async {
        wait_until_listening().await?;
        let mut client = Client::connect(&host_token).await?;
        let started = client
            .call(&Request::StartClaude {
                token: token.to_string(),
                allow_tools: false,
                hold_permissions: true,
            })
            .await?;
        let Response::Started { run_id, cursor, .. } = started else {
            anyhow::bail!("Claude did not start: {started:?}");
        };
        let prompt_id = format!(
            "held-{}",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        );
        let mut writer = Client::connect(&host_token).await?;
        let follow = tokio::spawn(async move {
            let _ = writer
                .call(&Request::Follow {
                    id: prompt_id,
                    text: PERM_TOOL.into(),
                })
                .await;
        });
        let permission_id = wait_permission(&mut client, Duration::from_secs(180)).await?;
        follow.abort();
        drop(client);
        tokio::time::sleep(Duration::from_secs(2)).await;
        let mut client = Client::connect(&host_token).await?;
        match client.call(&Request::Status).await? {
            Response::Status {
                run_id: Some(id),
                running: true,
                permission_id: Some(still),
                waiting: Some(waiting),
                ..
            } if id == run_id && still == permission_id && waiting == "permission" => {}
            other => anyhow::bail!("permission did not survive the disconnect: {other:?}"),
        }
        let during = texts_after(&mut client, cursor).await?;
        if during.iter().any(|text| text.contains("perm:allowed:")) {
            anyhow::bail!("permission was granted while the client was gone");
        }
        match client
            .call(&Request::Permit {
                id: permission_id.clone(),
                choice: "allow".into(),
            })
            .await?
        {
            Response::Ack { outcome, .. } if outcome == "delivered" => {}
            other => anyhow::bail!("permit was {other:?}"),
        }
        let deadline = Instant::now() + Duration::from_secs(150);
        loop {
            if Instant::now() > deadline {
                anyhow::bail!("tool did not finish after the permission was allowed");
            }
            let texts = texts_after(&mut client, cursor).await?;
            let allowed = texts
                .iter()
                .any(|text| text.contains(&format!("perm:allowed:{permission_id}")));
            let finished = texts.iter().any(|text| text.contains("stop end_turn"));
            if allowed && finished {
                println!(
                    "PASS Claude permission {permission_id} held across disconnect on {}",
                    engine.label
                );
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
    .await;
    let err = controller_log(&dir);
    shutdown(child);
    cleanup(&engine.host);
    outcome.map_err(|err_value| {
        if err.is_empty() {
            err_value
        } else {
            anyhow::anyhow!("{err_value}\n{err}")
        }
    })
}

async fn wait_permission(client: &mut Client, budget: Duration) -> anyhow::Result<String> {
    let deadline = Instant::now() + budget;
    loop {
        if Instant::now() > deadline {
            anyhow::bail!("Claude did not ask for permission");
        }
        if let Response::Status {
            permission_id: Some(id),
            waiting: Some(waiting),
            running: true,
            ..
        } = client.call(&Request::Status).await?
        {
            if waiting == "permission" {
                return Ok(id);
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn texts_after(client: &mut Client, after: u64) -> anyhow::Result<Vec<String>> {
    let mut texts = Vec::new();
    let mut cursor = after;
    loop {
        match client.call(&Request::Replay { after: cursor }).await? {
            Response::Replay {
                events,
                cursor: next,
                truncated,
                ..
            } => {
                let last = events.last().map(|event| event.seq);
                texts.extend(events.into_iter().map(|event| event.text));
                if !truncated {
                    return Ok(texts);
                }
                cursor = last.unwrap_or(next);
            }
            Response::Err { message } => anyhow::bail!("replay: {message}"),
            other => anyhow::bail!("unexpected replay {other:?}"),
        }
    }
}

async fn wait_until_listening() -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if TcpStream::connect(("127.0.0.1", PORT)).await.is_ok() {
            return Ok(());
        }
        if Instant::now() > deadline {
            anyhow::bail!("controller did not listen");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn ensure_image(host: &str, name: &str, relative: &str) -> anyhow::Result<()> {
    let inspect = Command::new("docker")
        .env("DOCKER_HOST", host)
        .args(["image", "inspect", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if inspect.success() {
        return Ok(());
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    let status = Command::new("docker")
        .env("DOCKER_HOST", host)
        .args(["build", "-t", name])
        .arg(&dir)
        .status()?;
    if !status.success() {
        anyhow::bail!("could not build {name}");
    }
    Ok(())
}

fn state_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join("brainiac-spike-local").join(name)
}

fn write_token(dir: &Path) -> anyhow::Result<String> {
    fs::create_dir_all(dir)?;
    let mut bytes = [0u8; 32];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    let token = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let path = dir.join("token");
    fs::write(&path, &token)?;
    let mut perms = fs::metadata(&path)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(&path, perms)?;
    Ok(token)
}

fn spawn_controller(host: &str, dir: &Path) -> anyhow::Result<std::process::Child> {
    let log = File::create(dir.join("controller.err"))?;
    let child = Command::new(std::env::current_exe()?)
        .args(["controller", "--state"])
        .arg(dir)
        .env("DOCKER_HOST", host)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log))
        .spawn()?;
    Ok(child)
}

fn controller_log(dir: &Path) -> String {
    fs::read_to_string(dir.join("controller.err")).unwrap_or_default()
}

fn engine_banner(dir: &Path) -> String {
    controller_log(dir)
        .lines()
        .find_map(|line| line.split_once("engine=").map(|(_, banner)| banner.trim()))
        .unwrap_or("unconfirmed")
        .to_string()
}

fn shutdown(mut child: std::process::Child) {
    let _ = Command::new("kill").arg(child.id().to_string()).status();
    let start = Instant::now();
    loop {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        if start.elapsed() > Duration::from_secs(8) {
            let _ = child.kill();
            let _ = child.wait();
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn tick_count(host: &str) -> anyhow::Result<u64> {
    let output = docker(host, &["logs", "brainiac-spike-lid-tick"])?;
    Ok(output
        .lines()
        .filter(|line| line.starts_with("tick-"))
        .count() as u64)
}

fn cleanup(host: &str) {
    if let Ok(ids) = docker(host, &["ps", "-aq", "--filter", "label=brainiac.spike=1"]) {
        for id in ids.split_whitespace() {
            let _ = docker(host, &["rm", "-f", id]);
        }
    }
    if let Ok(vols) = docker(
        host,
        &["volume", "ls", "-q", "--filter", "name=brainiac-spike-"],
    ) {
        for vol in vols.split_whitespace() {
            let _ = docker(host, &["volume", "rm", "-f", vol]);
        }
    }
}

fn docker(host: &str, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("docker")
        .env("DOCKER_HOST", host)
        .args(args)
        .output()?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.status.success() {
        text.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    Ok(text)
}
