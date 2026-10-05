//! Mac-side checks. The host name comes from `SPIKE_SSH` and is never written
//! into the repository. Sleep is `hold`; everything else runs unattended.

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;

use crate::messages::{Event, Request, Response};
use crate::state::PORT;

const STATE: &str = "/var/lib/brainiac-spike";

struct Tunnel {
    child: Child,
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
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

pub async fn run(section: Option<&str>) -> anyhow::Result<()> {
    match section {
        None | Some("all") => {
            session().await?;
            limits().await?;
            export_in_container().await?;
            remote_script("probe-quota.sh").await?;
            remote_script("collect-volume.sh").await?;
            interrupt().await?;
        }
        Some("session") => session().await?,
        Some("limit") => limits().await?,
        Some("export") => export_in_container().await?,
        Some("quota") => remote_script("probe-quota.sh").await?,
        Some("collect") => remote_script("collect-volume.sh").await?,
        Some("interrupt") => interrupt().await?,
        Some("claude") => claude().await?,
        Some(other) => anyhow::bail!("unknown prove section {other}"),
    }
    Ok(())
}

pub async fn hold() -> anyhow::Result<()> {
    let target = ssh_target()?;
    let token = token(&target)?;
    let _tunnel = open_tunnel(&target)?;
    wait_until_listening().await?;
    let mut client = Client::connect(&token).await?;
    if !running(&mut client).await? {
        expect_started(&mut client).await?;
    }
    let before = cursor_and_run(&mut client).await?;
    println!(
        "session {} is running at cursor {}. Leave this process open and sleep the Mac for at least 20 seconds.",
        before.0, before.1
    );
    let wall = SystemTime::now();
    let mono = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let wall_elapsed = wall.elapsed().unwrap_or_default();
        let mono_elapsed = mono.elapsed();
        if wall_elapsed > mono_elapsed + Duration::from_secs(15) {
            println!(
                "wake detected after about {} seconds asleep",
                (wall_elapsed.saturating_sub(mono_elapsed)).as_secs()
            );
            break;
        }
    }
    let mut client = reconnect(&target, &token).await?;
    let after = cursor_and_run(&mut client).await?;
    if after.0 != before.0 {
        anyhow::bail!("run id changed across sleep: {} then {}", before.0, after.0);
    }
    let beats = beats_after(&mut client, before.1).await?;
    println!(
        "same session {}, {} beats journaled while the Mac was asleep",
        after.0, beats
    );
    if beats < 10 {
        anyhow::bail!("expected the host to keep journaling through sleep, saw {beats} beats");
    }
    println!("PASS sleep");
    Ok(())
}

const SLEEP_TOOL: &str = "Use the Bash tool to run this command, wait until it finishes, and do nothing else: i=1; while [ $i -le 90 ]; do echo tick-$i >> /workspace/ticks; i=$((i+1)); sleep 1; done; echo spike-awake >> /workspace/ticks";

pub async fn hold_claude() -> anyhow::Result<()> {
    let subscription = std::env::var("SPIKE_CLAUDE_TOKEN")
        .map_err(|_| anyhow::anyhow!("set SPIKE_CLAUDE_TOKEN to the claude setup-token value"))?;
    if !crate::acp::valid_token(&subscription) {
        anyhow::bail!("SPIKE_CLAUDE_TOKEN is not a single printable line");
    }
    let target = ssh_target()?;
    remote(
        &target,
        &[
            "sudo",
            "-n",
            "systemctl",
            "restart",
            "brainiac-spike-controller",
        ],
    )?;
    let host_token = token(&target)?;
    let tunnel = open_tunnel(&target)?;
    wait_until_listening().await?;
    let mut client = Client::connect(&host_token).await?;
    // bootstrap:ready is journaled before the start event, so the cursor
    // has to be taken first.
    let before_start = current_cursor(&mut client).await?;
    discard_kept(&mut client).await?;
    let run_id = match client
        .call(&Request::StartClaude {
            token: subscription,
            allow_tools: true,
            hold_permissions: false,
        })
        .await?
    {
        Response::Started { run_id, .. } => run_id,
        Response::Err { message } => anyhow::bail!("start failed: {message}"),
        other => anyhow::bail!("unexpected start response {other:?}"),
    };
    println!("session {run_id} is up. Waiting until the tool is writing. Do not sleep yet.");
    let prompt_id = format!(
        "sleep-{}",
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    );
    let mut writer = Client::connect(&host_token).await?;
    tokio::spawn(async move {
        let _ = writer
            .call(&Request::Follow {
                id: prompt_id,
                text: SLEEP_TOOL.into(),
            })
            .await;
    });
    let lines_before = wait_ticks(&target, 1, 150).await?;
    if lines_before > 20 {
        anyhow::bail!("the tool had already written {lines_before} lines before the sleep window");
    }
    println!();
    println!("SLEEP NOW. Close the lid for 20 to 30 seconds. Leave this terminal running.");
    println!("The container is writing one line a second ({lines_before} so far).");
    println!();
    drop(tunnel);
    drop(client);
    let told = Instant::now();
    let wall = SystemTime::now();
    let mono = Instant::now();
    let slept = loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let wall_elapsed = wall.elapsed().unwrap_or_default();
        let mono_elapsed = mono.elapsed();
        if wall_elapsed > mono_elapsed + Duration::from_secs(15) {
            let slept = wall_elapsed.saturating_sub(mono_elapsed);
            println!(
                "wake detected after about {} seconds asleep",
                slept.as_secs()
            );
            break slept;
        }
    };
    if slept < Duration::from_secs(20) {
        anyhow::bail!(
            "sleep was only {} seconds; close the lid for at least 20",
            slept.as_secs()
        );
    }
    let lines_after = wait_ticks(&target, lines_before, 40).await?;
    // Instant does not advance while the Mac is suspended, so this is the
    // time the Mac was awake. The tool writes one line a second, so lines
    // beyond that interval were written during the sleep.
    let awake = told.elapsed().as_secs();
    let added = lines_after.saturating_sub(lines_before) as u64;
    println!(
        "ticks {lines_before} when sleep was requested, {lines_after} after wake, {added} written, {awake}s awake, {}s asleep",
        slept.as_secs()
    );
    if added <= awake {
        anyhow::bail!("the tool did not keep writing through sleep");
    }
    let _tunnel = open_tunnel(&target)?;
    wait_until_listening().await?;
    let mut client = Client::connect(&host_token).await?;
    let (run_again, _) = cursor_and_run(&mut client).await?;
    if run_again != run_id {
        anyhow::bail!("run id changed across sleep: {run_id} then {run_again}");
    }
    let ready = texts_after(&mut client, before_start)
        .await?
        .iter()
        .filter(|event| event.text.contains("bootstrap:ready"))
        .count();
    if ready != 1 {
        anyhow::bail!("credential frame was delivered {ready} times");
    }
    println!("same session {run_id}, token delivered once");
    let gamma_id = format!(
        "gamma-{}",
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    );
    println!("sending a follow-up on the same session");
    let cursor = current_cursor(&mut client).await?;
    expect_ack(
        &client
            .call(&Request::Follow {
                id: gamma_id,
                text: "Reply with exactly gamma-spike and nothing else.".into(),
            })
            .await?,
        "delivered",
    )?;
    wait_text(&mut client, cursor, "gamma-spike", 180).await?;
    println!("PASS claude sleep");
    Ok(())
}

async fn wait_ticks(target: &str, at_least: usize, secs: u64) -> anyhow::Result<usize> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut last = 0usize;
    while Instant::now() < deadline {
        if let Some(count) = tick_count(target) {
            last = count;
            if count >= at_least {
                return Ok(count);
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    anyhow::bail!("tick file stayed at {last}, wanted at least {at_least}")
}

fn tick_count(target: &str) -> Option<usize> {
    let id = remote_text(
        target,
        &[
            "sudo",
            "-n",
            "docker",
            "ps",
            "-q",
            "--filter",
            "label=brainiac.spike=1",
        ],
    )
    .ok()?;
    let id = id.trim();
    if id.is_empty() || id.contains('\n') || !id.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    let output = remote_text(
        target,
        &[
            "sudo",
            "-n",
            "docker",
            "exec",
            id,
            "wc",
            "-l",
            "/workspace/ticks",
        ],
    )
    .ok()?;
    output.split_whitespace().next()?.parse().ok()
}

async fn session() -> anyhow::Result<()> {
    let target = ssh_target()?;
    remote(
        &target,
        &["sudo", "-n", "systemctl", "restart", "brainiac-spike-guard"],
    )?;
    remote(
        &target,
        &[
            "sudo",
            "-n",
            "systemctl",
            "restart",
            "brainiac-spike-controller",
        ],
    )?;
    let token = token(&target)?;
    let tunnel = open_tunnel(&target)?;
    wait_until_listening().await?;
    let mut client = Client::connect(&token).await?;
    let started = expect_started(&mut client).await?;
    if started.tty || started.log_driver != "none" {
        anyhow::bail!("tty={} log={}", started.tty, started.log_driver);
    }
    println!("PASS non-tty log={}", started.log_driver);
    remote(
        &target,
        &[
            "sudo",
            "-n",
            "bash",
            &format!("{STATE}/src/scripts/check-logs.sh"),
        ],
    )?;
    println!("PASS raw logs disabled");

    let first = wait_beat(&mut client, 0).await?;
    drop(client);
    tokio::time::sleep(Duration::from_secs(3)).await;
    let mut client = Client::connect(&token).await?;
    let (run_id, cursor) = cursor_and_run(&mut client).await?;
    if run_id != started.run_id {
        anyhow::bail!("reconnect changed the run id");
    }
    if cursor <= first.seq {
        anyhow::bail!("journal did not advance while the client was gone");
    }
    println!(
        "PASS reconnect same session, cursor {first_seq} -> {cursor}",
        first_seq = first.seq
    );

    // Drop the tunnel, which is what an SSH failure looks like to the Mac.
    // The controller is not a child of SSH, so the attach stays up.
    drop(tunnel);
    tokio::time::sleep(Duration::from_secs(3)).await;
    let tunnel = open_tunnel(&target)?;
    wait_until_listening().await?;
    let mut client = Client::connect(&token).await?;
    let (run_again, cursor_again) = cursor_and_run(&mut client).await?;
    if run_again != started.run_id || cursor_again <= cursor {
        anyhow::bail!("SSH loss did not keep the same session journaling");
    }
    println!("PASS SSH loss, cursor -> {cursor_again}");

    let follow = client
        .call(&Request::Follow {
            id: "marker-1".into(),
            text: "marker-1".into(),
        })
        .await?;
    expect_ack(&follow, "delivered")?;
    let again = client
        .call(&Request::Follow {
            id: "marker-1".into(),
            text: "marker-1-again".into(),
        })
        .await?;
    expect_ack(&again, "delivered")?;
    let echoes = wait_follows(&mut client, cursor_again, "follow:marker-1").await?;
    if echoes != 1 {
        anyhow::bail!("duplicate command id produced {echoes} follow-ups");
    }
    println!("PASS duplicate command id was not sent twice");

    remote(
        &target,
        &[
            "/usr/local/bin/brainiac-spike",
            "emergency-stop",
            "--state",
            STATE,
        ],
    )?;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let running = remote_text(
        &target,
        &[
            "sudo",
            "-n",
            "docker",
            "ps",
            "-q",
            "--filter",
            "label=brainiac.spike=1",
        ],
    )?;
    if !running.trim().is_empty() {
        anyhow::bail!("emergency stop left a container running");
    }
    let active = remote_text(
        &target,
        &["systemctl", "is-active", "brainiac-spike-controller"],
    )?;
    if active.trim() != "active" {
        anyhow::bail!("emergency stop stopped the controller");
    }
    let kept = remote_text(
        &target,
        &[
            "sudo",
            "-n",
            "docker",
            "ps",
            "-aq",
            "--filter",
            "label=brainiac.spike=1",
        ],
    )?;
    if kept.trim().is_empty() {
        anyhow::bail!("emergency stop removed the container instead of stopping it");
    }
    println!("PASS emergency stop, container kept");

    // A new session, then kill the controller. The guard has to stop the
    // container; Restart=no means systemd leaves the controller dead.
    expect_started(&mut client).await?;
    let pid = remote_text(&target, &["cat", &format!("{STATE}/controller.pid")])?;
    remote(&target, &["kill", "-9", pid.trim()])?;
    let mut stopped = false;
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let ps = remote_text(
            &target,
            &[
                "sudo",
                "-n",
                "docker",
                "ps",
                "-q",
                "--filter",
                "label=brainiac.spike=1",
            ],
        )?;
        if ps.trim().is_empty() {
            stopped = true;
            break;
        }
    }
    if !stopped {
        anyhow::bail!("guard did not stop the container after the controller was killed");
    }
    let controller = remote_text(
        &target,
        &["systemctl", "is-active", "brainiac-spike-controller"],
    )
    .unwrap_or_default();
    if controller.trim() == "active" {
        anyhow::bail!("controller was restarted after kill -9");
    }
    println!("PASS controller crash stops the workload");

    remote(
        &target,
        &[
            "sudo",
            "-n",
            "systemctl",
            "start",
            "brainiac-spike-controller",
        ],
    )?;
    drop(client);
    drop(tunnel);
    let _tunnel = open_tunnel(&target)?;
    wait_until_listening().await?;
    let mut client = Client::connect(&token).await?;
    match client.call(&Request::Status).await? {
        Response::Status {
            running,
            interrupted,
            ..
        } => {
            if running {
                anyhow::bail!("restarted controller adopted the old session");
            }
            if !interrupted {
                anyhow::bail!("restarted controller did not record the interruption");
            }
        }
        other => anyhow::bail!("unexpected status {other:?}"),
    }
    println!("PASS restart does not resume the killed session");
    Ok(())
}

async fn limits() -> anyhow::Result<()> {
    permission_held().await?;
    deadline_while_waiting().await?;
    deadline_not_extended().await?;
    Ok(())
}

async fn permission_held() -> anyhow::Result<()> {
    let target = ssh_target()?;
    restart_controller(&target)?;
    let host_token = token(&target)?;
    let tunnel = open_tunnel(&target)?;
    wait_until_listening().await?;
    let mut client = Client::connect(&host_token).await?;
    let started = start_with(&mut client, 120).await?;
    let cursor = current_cursor(&mut client).await?;
    expect_ack(
        &client
            .call(&Request::Follow {
                id: "ask-1".into(),
                text: "permission".into(),
            })
            .await?,
        "delivered",
    )?;
    let pending = wait_text(&mut client, cursor, "perm:pending:", 8).await?;
    let id = pending
        .text
        .strip_prefix("perm:pending:")
        .unwrap_or("")
        .to_string();
    if id.is_empty() {
        anyhow::bail!("permission id was empty");
    }
    let snap = snapshot(&mut client).await?;
    if snap.run_id.as_deref() != Some(started.run_id.as_str())
        || snap.waiting.as_deref() != Some("permission")
        || snap.permission_id.as_deref() != Some(id.as_str())
    {
        anyhow::bail!("permission was not waiting: {snap:?}");
    }
    match client
        .call(&Request::Follow {
            id: "ask-during".into(),
            text: "nope".into(),
        })
        .await?
    {
        Response::Err { message } if message.contains("permission") => {}
        other => anyhow::bail!("follow during a permission was {other:?}"),
    }

    drop(tunnel);
    tokio::time::sleep(Duration::from_secs(3)).await;
    let _tunnel = open_tunnel(&target)?;
    wait_until_listening().await?;
    let mut client = Client::connect(&host_token).await?;
    let again = snapshot(&mut client).await?;
    if again.run_id.as_deref() != Some(started.run_id.as_str())
        || !again.running
        || again.permission_id.as_deref() != Some(id.as_str())
    {
        anyhow::bail!("permission did not survive SSH loss: {again:?}");
    }
    if texts_after(&mut client, cursor)
        .await?
        .iter()
        .any(|event| event.text.contains("perm:allowed:"))
    {
        anyhow::bail!("permission was granted while the client was gone");
    }
    println!(
        "PASS permission {id} held across SSH loss on {}",
        started.run_id
    );

    expect_ack(
        &client
            .call(&Request::Permit {
                id: id.clone(),
                choice: "allow".into(),
            })
            .await?,
        "delivered",
    )?;
    expect_ack(
        &client
            .call(&Request::Permit {
                id: id.clone(),
                choice: "allow".into(),
            })
            .await?,
        "delivered",
    )?;
    let granted = format!("perm:allowed:{id}");
    wait_text(&mut client, cursor, &granted, 8).await?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let grants = texts_after(&mut client, cursor)
        .await?
        .iter()
        .filter(|event| event.text == granted)
        .count();
    if grants != 1 {
        anyhow::bail!("permission {id} was granted {grants} times");
    }
    println!("PASS permission {id} answered once");

    let cursor = current_cursor(&mut client).await?;
    expect_ack(
        &client
            .call(&Request::Follow {
                id: "ask-2".into(),
                text: "permission".into(),
            })
            .await?,
        "delivered",
    )?;
    let second = wait_text(&mut client, cursor, "perm:pending:", 8).await?;
    let id2 = second
        .text
        .strip_prefix("perm:pending:")
        .unwrap_or("")
        .to_string();
    expect_ack(
        &client
            .call(&Request::Permit {
                id: id2.clone(),
                choice: "cancel".into(),
            })
            .await?,
        "delivered",
    )?;
    let cancelled = format!("perm:cancelled:{id2}");
    let cancel_event = wait_text(&mut client, cursor, &cancelled, 8).await?;
    expect_ack(
        &client
            .call(&Request::Permit {
                id: id2.clone(),
                choice: "allow".into(),
            })
            .await?,
        "delivered",
    )?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    let after = texts_after(&mut client, cursor).await?;
    if after
        .iter()
        .any(|event| event.text == format!("perm:allowed:{id2}"))
    {
        anyhow::bail!("a late allow granted {id2}");
    }
    if after.iter().filter(|event| event.text == cancelled).count() != 1 {
        anyhow::bail!("cancel was not recorded once for {id2}");
    }
    let snap = snapshot(&mut client).await?;
    if !snap.running || snap.waiting.as_deref() != Some("idle") {
        anyhow::bail!("cancel stopped the session: {snap:?}");
    }
    wait_text(&mut client, cancel_event.seq, "beat:", 5).await?;
    println!("PASS cancel of {id2}; a later allow had no effect");
    Ok(())
}

async fn deadline_while_waiting() -> anyhow::Result<()> {
    let target = ssh_target()?;
    restart_controller(&target)?;
    let host_token = token(&target)?;
    let tunnel = open_tunnel(&target)?;
    wait_until_listening().await?;
    let mut client = Client::connect(&host_token).await?;
    let started = start_with(&mut client, 8).await?;
    let cursor = current_cursor(&mut client).await?;
    expect_ack(
        &client
            .call(&Request::Follow {
                id: "ask-expire".into(),
                text: "permission".into(),
            })
            .await?,
        "delivered",
    )?;
    let pending = wait_text(&mut client, cursor, "perm:pending:", 5).await?;
    let id = pending
        .text
        .strip_prefix("perm:pending:")
        .unwrap_or("")
        .to_string();
    let before = snapshot(&mut client).await?;
    if !before.running || before.permission_id.as_deref() != Some(id.as_str()) {
        anyhow::bail!("permission was not pending when the client left: {before:?}");
    }
    drop(tunnel);
    tokio::time::sleep(Duration::from_secs(10)).await;
    let _tunnel = open_tunnel(&target)?;
    wait_until_listening().await?;
    let mut client = Client::connect(&host_token).await?;
    let snap = wait_marked_expired(&mut client, Duration::from_secs(8)).await?;
    if snap.interrupted || snap.run_id.as_deref() != Some(started.run_id.as_str()) {
        anyhow::bail!("expiry looked like an interruption: {snap:?}");
    }
    let events = texts_after(&mut client, cursor).await?;
    if !events
        .iter()
        .any(|event| event.text == format!("cancelled {id}"))
        || !events.iter().any(|event| event.text == "expired")
        || events
            .iter()
            .any(|event| event.text.contains("perm:allowed:"))
    {
        anyhow::bail!("deadline did not cancel the permission");
    }
    wait_not_running(&mut client, Duration::from_secs(15)).await?;
    assert_container_gone(&target).await?;
    match client
        .call(&Request::Permit {
            id,
            choice: "allow".into(),
        })
        .await?
    {
        Response::Err { .. } => {}
        other => anyhow::bail!("allow after expiry had an effect: {other:?}"),
    }
    println!(
        "PASS deadline cancelled a permission while the client was gone on {}",
        started.run_id
    );
    Ok(())
}

async fn deadline_not_extended() -> anyhow::Result<()> {
    let target = ssh_target()?;
    restart_controller(&target)?;
    let host_token = token(&target)?;
    let tunnel = open_tunnel(&target)?;
    wait_until_listening().await?;
    let mut client = Client::connect(&host_token).await?;
    let started = start_with(&mut client, 15).await?;
    let armed = Instant::now();
    tokio::time::sleep(Duration::from_secs(4)).await;
    if !running(&mut client).await? {
        anyhow::bail!("session stopped before the disconnect");
    }
    drop(tunnel);
    tokio::time::sleep(Duration::from_secs(4)).await;
    if armed.elapsed() > Duration::from_secs(11) {
        anyhow::bail!("reconnect check ran too late to distinguish a reset deadline");
    }
    let _tunnel = open_tunnel(&target)?;
    wait_until_listening().await?;
    let mut client = Client::connect(&host_token).await?;
    let snap = snapshot(&mut client).await?;
    if !snap.running || snap.run_id.as_deref() != Some(started.run_id.as_str()) {
        anyhow::bail!("session was gone before its original deadline: {snap:?}");
    }
    let back = Instant::now();
    wait_marked_expired(&mut client, Duration::from_secs(8)).await?;
    let since_reconnect = back.elapsed();
    if since_reconnect >= Duration::from_secs(8) {
        anyhow::bail!(
            "deadline was not marked within 8s of reconnect; it appears to have been restarted"
        );
    }
    wait_not_running(&mut client, Duration::from_secs(15)).await?;
    assert_container_gone(&target).await?;
    println!(
        "PASS reconnect does not extend the deadline ({since}s after reconnect, {total}s from start)",
        since = since_reconnect.as_secs(),
        total = armed.elapsed().as_secs()
    );
    Ok(())
}

fn restart_controller(target: &str) -> anyhow::Result<()> {
    remote(
        target,
        &["sudo", "-n", "systemctl", "restart", "brainiac-spike-guard"],
    )?;
    remote(
        target,
        &[
            "sudo",
            "-n",
            "systemctl",
            "restart",
            "brainiac-spike-controller",
        ],
    )
}

#[derive(Debug)]
struct Snapshot {
    run_id: Option<String>,
    running: bool,
    waiting: Option<String>,
    permission_id: Option<String>,
    expired: bool,
    interrupted: bool,
}

async fn snapshot(client: &mut Client) -> anyhow::Result<Snapshot> {
    match client.call(&Request::Status).await? {
        Response::Status {
            run_id,
            running,
            waiting,
            permission_id,
            expired,
            interrupted,
            ..
        } => Ok(Snapshot {
            run_id,
            running,
            waiting,
            permission_id,
            expired,
            interrupted,
        }),
        Response::Err { message } => anyhow::bail!("{message}"),
        other => anyhow::bail!("unexpected status {other:?}"),
    }
}

async fn wait_text(
    client: &mut Client,
    after: u64,
    needle: &str,
    secs: u64,
) -> anyhow::Result<Event> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut cursor = after;
    while Instant::now() < deadline {
        for event in texts_after(client, cursor).await? {
            cursor = cursor.max(event.seq);
            if event.text.contains(needle) {
                return Ok(event);
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    anyhow::bail!("timed out waiting for {needle}")
}

async fn texts_after(client: &mut Client, after: u64) -> anyhow::Result<Vec<Event>> {
    let mut cursor = after;
    let mut events = Vec::new();
    loop {
        match client.call(&Request::Replay { after: cursor }).await? {
            Response::Replay {
                events: page,
                truncated,
                ..
            } => {
                if page.is_empty() {
                    break;
                }
                cursor = page.last().map(|event| event.seq).unwrap_or(cursor);
                let more = truncated;
                events.extend(page);
                if !more {
                    break;
                }
            }
            Response::Err { message } => anyhow::bail!("{message}"),
            other => anyhow::bail!("unexpected replay {other:?}"),
        }
    }
    Ok(events)
}

async fn wait_marked_expired(client: &mut Client, budget: Duration) -> anyhow::Result<Snapshot> {
    let deadline = Instant::now() + budget;
    loop {
        let snap = snapshot(client).await?;
        if snap.expired {
            return Ok(snap);
        }
        if Instant::now() >= deadline {
            anyhow::bail!("deadline was not recorded: {snap:?}");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn wait_not_running(client: &mut Client, budget: Duration) -> anyhow::Result<()> {
    let deadline = Instant::now() + budget;
    loop {
        let snap = snapshot(client).await?;
        if !snap.running {
            return Ok(());
        }
        if Instant::now() >= deadline {
            anyhow::bail!("container was still attached after expiry: {snap:?}");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn assert_container_gone(target: &str) -> anyhow::Result<()> {
    for _ in 0..20 {
        let ps = remote_text(
            target,
            &[
                "sudo",
                "-n",
                "docker",
                "ps",
                "-q",
                "--filter",
                "label=brainiac.spike=1",
            ],
        )?;
        if ps.trim().is_empty() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    anyhow::bail!("a spike container was still running")
}

struct Started {
    run_id: String,
    cursor: u64,
    tty: bool,
    log_driver: String,
}

async fn expect_started(client: &mut Client) -> anyhow::Result<Started> {
    start_with(client, 600).await
}

async fn start_with(client: &mut Client, deadline_secs: u64) -> anyhow::Result<Started> {
    discard_kept(client).await?;
    match client.call(&Request::Start { deadline_secs }).await? {
        Response::Started {
            run_id,
            cursor,
            tty,
            log_driver,
        } => Ok(Started {
            run_id,
            cursor,
            tty,
            log_driver,
        }),
        Response::Err { message } => anyhow::bail!("start failed: {message}"),
        other => anyhow::bail!("unexpected start response {other:?}"),
    }
}

async fn running(client: &mut Client) -> anyhow::Result<bool> {
    match client.call(&Request::Status).await? {
        Response::Status { running, .. } => Ok(running),
        Response::Err { message } => anyhow::bail!("{message}"),
        other => anyhow::bail!("unexpected status {other:?}"),
    }
}

async fn cursor_and_run(client: &mut Client) -> anyhow::Result<(String, u64)> {
    match client.call(&Request::Status).await? {
        Response::Status {
            run_id: Some(run_id),
            cursor,
            running: true,
            ..
        } => Ok((run_id, cursor)),
        Response::Status { running: false, .. } => anyhow::bail!("session is not running"),
        Response::Err { message } => anyhow::bail!("{message}"),
        other => anyhow::bail!("unexpected status {other:?}"),
    }
}

async fn wait_beat(client: &mut Client, after: u64) -> anyhow::Result<Event> {
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut cursor = after;
    while Instant::now() < deadline {
        if let Some(event) = find_event(client, cursor, |event| event.kind == "beat").await? {
            return Ok(event);
        }
        cursor = current_cursor(client).await?;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    anyhow::bail!("no heartbeat arrived")
}

async fn wait_follows(client: &mut Client, after: u64, text: &str) -> anyhow::Result<usize> {
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut seen = 0usize;
    let mut cursor = after;
    while Instant::now() < deadline {
        match client.call(&Request::Replay { after: cursor }).await? {
            Response::Replay { events, .. } => {
                for event in events {
                    if event.kind == "follow" {
                        if event.text != text {
                            anyhow::bail!("unexpected follow-up {}", event.text);
                        }
                        seen += 1;
                    }
                    cursor = cursor.max(event.seq);
                }
            }
            Response::Err { message } => anyhow::bail!("{message}"),
            other => anyhow::bail!("unexpected replay {other:?}"),
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    Ok(seen)
}

async fn find_event(
    client: &mut Client,
    after: u64,
    pred: impl Fn(&Event) -> bool,
) -> anyhow::Result<Option<Event>> {
    match client.call(&Request::Replay { after }).await? {
        Response::Replay { events, .. } => Ok(events.into_iter().find(pred)),
        Response::Err { message } => anyhow::bail!("{message}"),
        other => anyhow::bail!("unexpected replay {other:?}"),
    }
}

async fn current_cursor(client: &mut Client) -> anyhow::Result<u64> {
    match client.call(&Request::Status).await? {
        Response::Status { cursor, .. } => Ok(cursor),
        other => anyhow::bail!("unexpected status {other:?}"),
    }
}

async fn beats_after(client: &mut Client, after: u64) -> anyhow::Result<usize> {
    let mut count = 0usize;
    let mut cursor = after;
    loop {
        match client.call(&Request::Replay { after: cursor }).await? {
            Response::Replay {
                events, truncated, ..
            } => {
                if events.is_empty() {
                    break;
                }
                for event in events {
                    if event.kind == "beat" {
                        count += 1;
                    }
                    cursor = cursor.max(event.seq);
                }
                if !truncated {
                    break;
                }
            }
            Response::Err { message } => anyhow::bail!("{message}"),
            other => anyhow::bail!("unexpected replay {other:?}"),
        }
    }
    Ok(count)
}

fn expect_ack(response: &Response, outcome: &str) -> anyhow::Result<()> {
    match response {
        Response::Ack { outcome: got, .. } if got == outcome => Ok(()),
        Response::Err { message } => anyhow::bail!("{message}"),
        other => anyhow::bail!("unexpected ack {other:?}"),
    }
}

async fn reconnect(target: &str, token: &str) -> anyhow::Result<Client> {
    // `hold` returns as soon as the check finishes, so the tunnel has to
    // outlive this function. The process exit closes it.
    let tunnel = open_tunnel(target)?;
    std::mem::forget(tunnel);
    wait_until_listening().await?;
    Client::connect(token).await
}

async fn wait_until_listening() -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", PORT)).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    anyhow::bail!("controller did not accept connections on 127.0.0.1:{PORT}")
}

fn open_tunnel(target: &str) -> anyhow::Result<Tunnel> {
    let child = Command::new("ssh")
        .args([
            "-N",
            "-o",
            "BatchMode=yes",
            "-o",
            "ExitOnForwardFailure=yes",
            "-o",
            "ConnectTimeout=8",
            "-L",
            &format!("127.0.0.1:{PORT}:127.0.0.1:{PORT}"),
            target,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(Tunnel { child })
}

async fn claude() -> anyhow::Result<()> {
    let subscription = std::env::var("SPIKE_CLAUDE_TOKEN")
        .map_err(|_| anyhow::anyhow!("set SPIKE_CLAUDE_TOKEN to the claude setup-token value"))?;
    if !crate::acp::valid_token(&subscription) {
        anyhow::bail!("SPIKE_CLAUDE_TOKEN is not a single printable line");
    }
    let target = ssh_target()?;
    remote(
        &target,
        &[
            "sudo",
            "-n",
            "systemctl",
            "restart",
            "brainiac-spike-controller",
        ],
    )?;
    let host_token = token(&target)?;
    let tunnel = open_tunnel(&target)?;
    wait_until_listening().await?;
    let mut client = Client::connect(&host_token).await?;
    let before = match client.call(&Request::Status).await? {
        Response::Status { cursor, .. } => cursor,
        Response::Err { message } => anyhow::bail!("{message}"),
        other => anyhow::bail!("unexpected status {other:?}"),
    };
    discard_kept(&mut client).await?;
    let started = match client
        .call(&Request::StartClaude {
            token: subscription.clone(),
            allow_tools: false,
            hold_permissions: false,
        })
        .await?
    {
        Response::Started {
            run_id,
            tty,
            log_driver,
            ..
        } => {
            if tty || log_driver != "none" {
                anyhow::bail!("tty={tty} log={log_driver}");
            }
            run_id
        }
        Response::Err { message } => {
            anyhow::bail!("start failed: {}", hide(&message, &subscription))
        }
        other => anyhow::bail!("unexpected start response {other:?}"),
    };
    println!("PASS claude session {started} tty=false log=none");

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let first = "Reply with exactly alpha-spike and no other words. Do not use tools.";
    let first_ack = client
        .call(&Request::Follow {
            id: format!("prompt-1-{stamp}"),
            text: first.into(),
        })
        .await?;
    match &first_ack {
        Response::Ack { outcome, .. } if outcome == "delivered" => {}
        Response::Err { message } => {
            anyhow::bail!("first prompt failed: {}", hide(message, &subscription))
        }
        other => anyhow::bail!("unexpected first prompt {other:?}"),
    }
    drop(client);
    drop(tunnel);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let _tunnel = open_tunnel(&target)?;
    wait_until_listening().await?;
    let mut client = Client::connect(&host_token).await?;
    let (run_id, _) = cursor_and_run(&mut client).await?;
    if run_id != started {
        anyhow::bail!("reconnect changed the run id");
    }
    let second = "Reply with exactly beta-spike and no other words. Do not use tools.";
    let second_ack = client
        .call(&Request::Follow {
            id: format!("prompt-2-{stamp}"),
            text: second.into(),
        })
        .await?;
    match &second_ack {
        Response::Ack { outcome, .. } if outcome == "delivered" => {}
        Response::Err { message } => {
            anyhow::bail!("second prompt failed: {}", hide(message, &subscription))
        }
        other => anyhow::bail!("unexpected second prompt {other:?}"),
    }
    println!("PASS two prompts on {run_id} across a dropped connection");

    let events = all_events(&mut client)
        .await?
        .into_iter()
        .filter(|event| event.seq > before)
        .collect::<Vec<_>>();
    if events
        .iter()
        .any(|event| event.text.contains(&subscription))
    {
        anyhow::bail!("credential appeared in the journal");
    }
    let ready = events
        .iter()
        .filter(|event| event.text == "bootstrap:ready")
        .count();
    let sessions = events
        .iter()
        .filter(|event| {
            event.text.starts_with("session ") || event.text.starts_with("claude-session ")
        })
        .count();
    if ready != 1 {
        anyhow::bail!("credential frame was delivered {ready} times");
    }
    if sessions < 1 {
        anyhow::bail!("ACP session id was not journaled");
    }
    let joined = events
        .iter()
        .map(|event| event.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if !joined.contains("alpha-spike") || !joined.contains("beta-spike") {
        anyhow::bail!("journal did not contain both replies");
    }
    println!("PASS same session, token delivered once");
    if events
        .iter()
        .any(|event| event.text.contains("bootstrap:onboarding-flag"))
    {
        println!("ONBOARDING flag written");
    } else {
        println!("ONBOARDING flag not observed");
    }
    if let Some(agent) = events.iter().find(|event| event.text.starts_with("agent ")) {
        println!("AGENT {}", agent.text);
    }

    let leaks = remote_text(
        &target,
        &[
            "sudo",
            "-n",
            "bash",
            &format!("{STATE}/src/scripts/check-claude-leaks.sh"),
        ],
    )?;
    if leaks.contains(&subscription) {
        anyhow::bail!("credential appeared in inspect, daemon logs, or the container home");
    }
    if !leaks.contains("TTY false")
        || !leaks.contains("LOG none")
        || !leaks.contains("RAWLOG empty")
    {
        anyhow::bail!("inspect did not show a non-TTY container with logging disabled");
    }
    let login = section(&leaks, "LOGIN_BEGIN", "LOGIN_END");
    if !login.is_empty() {
        anyhow::bail!("container home has a Claude login file");
    }
    let json = section(&leaks, "CLAUDE_JSON_BEGIN", "CLAUDE_JSON_END");
    let compact = json.replace([' ', '\n', '\t'], "");
    if !compact.contains("\"hasCompletedOnboarding\":true")
        || compact.contains("sk-ant-")
        || compact.contains("CLAUDE_CODE_OAUTH_TOKEN")
        || compact.contains("oauthToken")
    {
        anyhow::bail!("onboarding file was missing or contained the credential");
    }
    if let Some(reason) = json.split("cachedExtraUsageDisabledReason").nth(1) {
        let reason = reason.trim_start_matches(|ch: char| !ch.is_ascii_alphanumeric());
        let reason = reason.split('"').next().unwrap_or("");
        if !reason.is_empty() {
            println!("USAGE {reason}");
        }
    }
    let mounts = leaks
        .lines()
        .find(|line| line.starts_with("MOUNTS ") || line.starts_with("BINDS "))
        .unwrap_or("");
    if leaks.lines().any(|line| {
        (line.starts_with("MOUNTS ") || line.starts_with("BINDS "))
            && (line.contains("\"bind\"") || line.contains("\"Type\":\"bind\""))
    }) {
        anyhow::bail!("container has a bind mount: {mounts}");
    }
    println!("PASS credential absent from inspect, journal, daemon logs, and container home");
    for line in leaks.lines() {
        if line.starts_with("IMAGE ") || line.starts_with("LABELS ") || line.starts_with("USER ") {
            println!("{line}");
        }
    }

    remote(
        &target,
        &[
            "/usr/local/bin/brainiac-spike",
            "emergency-stop",
            "--state",
            STATE,
        ],
    )?;
    wait_until_stopped(&mut client).await?;
    let rejected = "sk-ant-oat01-spike-rejected-token";
    let mut client = Client::connect(&host_token).await?;
    discard_kept(&mut client).await?;
    let rejection = match client
        .call(&Request::StartClaude {
            token: rejected.into(),
            allow_tools: false,
            hold_permissions: false,
        })
        .await?
    {
        Response::Err { message } => message,
        Response::Started { .. } => {
            let prompt = client
                .call(&Request::Follow {
                    id: "rejected-1".into(),
                    text: "Reply with exactly reject-spike.".into(),
                })
                .await?;
            match prompt {
                Response::Err { message } => message,
                Response::Ack { .. } => "started and the prompt was accepted".into(),
                other => format!("unexpected rejected-token result {other:?}"),
            }
        }
        other => format!("unexpected rejected-token start {other:?}"),
    };
    let rejection = hide(&rejection, &subscription);
    println!("REJECTED {rejection}");
    let _ = remote(
        &target,
        &[
            "/usr/local/bin/brainiac-spike",
            "emergency-stop",
            "--state",
            STATE,
        ],
    );
    println!("PASS claude");
    Ok(())
}

fn hide(text: &str, secret: &str) -> String {
    text.replace(secret, "[credential]")
}

fn section<'a>(text: &'a str, begin: &str, end: &str) -> &'a str {
    let Some(start) = text.find(begin) else {
        return "";
    };
    let rest = &text[start + begin.len()..];
    let Some(stop) = rest.find(end) else {
        return rest;
    };
    rest[..stop].trim()
}

async fn all_events(client: &mut Client) -> anyhow::Result<Vec<Event>> {
    let mut cursor = 0u64;
    let mut out = Vec::new();
    loop {
        match client.call(&Request::Replay { after: cursor }).await? {
            Response::Replay {
                events, truncated, ..
            } => {
                if events.is_empty() {
                    break;
                }
                for event in events {
                    cursor = cursor.max(event.seq);
                    out.push(event);
                }
                if !truncated {
                    break;
                }
            }
            Response::Err { message } => anyhow::bail!("{message}"),
            other => anyhow::bail!("unexpected replay {other:?}"),
        }
    }
    Ok(out)
}

async fn wait_until_stopped(client: &mut Client) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        match client.call(&Request::Status).await? {
            Response::Status { running: false, .. } => return Ok(()),
            Response::Status { running: true, .. } => {
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            Response::Err { message } => anyhow::bail!("{message}"),
            other => anyhow::bail!("unexpected status {other:?}"),
        }
    }
    anyhow::bail!("session was still running after emergency stop")
}

/// The checks start many runs. Each discards the previous run's kept work
/// explicitly, the way a user would, because the controller refuses to
/// replace it.
async fn discard_kept(client: &mut Client) -> anyhow::Result<()> {
    match client.call(&Request::Discard).await? {
        Response::Ack { outcome, .. } if outcome == "discarded" => Ok(()),
        Response::Err { message } if message == "no kept work" => Ok(()),
        Response::Err { message } => anyhow::bail!("discard failed: {message}"),
        other => anyhow::bail!("unexpected discard response {other:?}"),
    }
}

async fn collect_entries(client: &mut Client) -> anyhow::Result<(String, Vec<String>)> {
    match client.call(&Request::Collect).await? {
        Response::Collected { run_id, entries } => Ok((run_id, entries)),
        Response::Err { message } => anyhow::bail!("collect failed: {message}"),
        other => anyhow::bail!("unexpected collect response {other:?}"),
    }
}

/// The text of one collected file, split into lines.
fn collected_lines(entries: &[String], path: &str) -> anyhow::Result<Vec<String>> {
    for entry in entries {
        let value: serde_json::Value = serde_json::from_str(entry)?;
        if value["path"] == path && value["kind"] == "file" {
            let text = value["text"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("{path} was collected without text"))?;
            return Ok(text.lines().map(str::to_string).collect());
        }
    }
    anyhow::bail!("{path} is missing from the collected work")
}

/// Interrupt a run twice, once by `kill -9` of the controller (the guard
/// stops the workload) and once by restarting the controller service. Each
/// time the stopped container and its volume must survive, a new start must
/// be refused, collection must hold every beat the journal recorded, and
/// only an explicit discard may remove the work.
async fn interrupt() -> anyhow::Result<()> {
    let target = ssh_target()?;
    restart_controller(&target)?;
    let token = token(&target)?;
    let mut tunnel = open_tunnel(&target)?;
    wait_until_listening().await?;
    let mut client = Client::connect(&token).await?;
    for (round, how) in ["kill -9", "service restart"].into_iter().enumerate() {
        let started = start_with(&mut client, 600).await?;
        let run = started.run_id.clone();
        let marker = format!("keep-{round}");
        expect_ack(
            &client
                .call(&Request::Follow {
                    id: marker.clone(),
                    text: marker.clone(),
                })
                .await?,
            "delivered",
        )?;
        wait_follows(&mut client, started.cursor, &format!("follow:{marker}")).await?;
        let deadline = Instant::now() + Duration::from_secs(20);
        while beats_after(&mut client, started.cursor).await? < 5 {
            if Instant::now() >= deadline {
                anyhow::bail!("the run did not beat five times");
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        if how == "kill -9" {
            let pid = remote_text(&target, &["cat", &format!("{STATE}/controller.pid")])?;
            remote(&target, &["kill", "-9", pid.trim()])?;
            assert_container_gone(&target).await?;
            remote(
                &target,
                &[
                    "sudo",
                    "-n",
                    "systemctl",
                    "start",
                    "brainiac-spike-controller",
                ],
            )?;
        } else {
            remote(
                &target,
                &[
                    "sudo",
                    "-n",
                    "systemctl",
                    "restart",
                    "brainiac-spike-controller",
                ],
            )?;
        }
        drop(client);
        drop(tunnel);
        tunnel = open_tunnel(&target)?;
        wait_until_listening().await?;
        client = Client::connect(&token).await?;

        match client.call(&Request::Status).await? {
            Response::Status {
                run_id,
                running,
                interrupted,
                kept,
                ..
            } => {
                if run_id.as_deref() != Some(run.as_str()) || running || !interrupted || !kept {
                    anyhow::bail!(
                        "{how}: expected run {run} interrupted and kept, got run={run_id:?} running={running} interrupted={interrupted} kept={kept}"
                    );
                }
            }
            other => anyhow::bail!("unexpected status {other:?}"),
        }
        let label = format!("label=brainiac.spike.run={run}");
        let all = remote_text(
            &target,
            &["sudo", "-n", "docker", "ps", "-aq", "--filter", &label],
        )?;
        let live = remote_text(
            &target,
            &["sudo", "-n", "docker", "ps", "-q", "--filter", &label],
        )?;
        let volumes = remote_text(
            &target,
            &[
                "sudo", "-n", "docker", "volume", "ls", "-q", "--filter", &label,
            ],
        )?;
        if all.trim().is_empty() || !live.trim().is_empty() || volumes.trim().is_empty() {
            anyhow::bail!(
                "{how}: expected a stopped container and its volume, got containers={:?} running={:?} volumes={:?}",
                all.trim(),
                live.trim(),
                volumes.trim()
            );
        }
        println!("PASS {how}: the workload is stopped, its container and volume are kept");

        match client.call(&Request::Start { deadline_secs: 600 }).await? {
            Response::Err { message } if message.contains("kept") => {}
            other => anyhow::bail!("{how}: a start replaced kept work: {other:?}"),
        }
        println!("PASS {how}: a new start is refused while the work is kept");

        let (collected_run, first) = collect_entries(&mut client).await?;
        let (_, second) = collect_entries(&mut client).await?;
        if collected_run != run {
            anyhow::bail!("{how}: collected run {collected_run}, expected {run}");
        }
        if first != second {
            anyhow::bail!("{how}: two collections of the same stopped volume differ");
        }
        let in_file = collected_lines(&first, "beats.txt")?;
        let journaled: Vec<String> = texts_after(&mut client, started.cursor)
            .await?
            .into_iter()
            .filter(|event| event.kind == "beat")
            .map(|event| event.text)
            .collect();
        let missing: Vec<&String> = journaled
            .iter()
            .filter(|beat| !in_file.contains(beat))
            .collect();
        if journaled.is_empty() || !missing.is_empty() {
            anyhow::bail!(
                "{how}: {} journaled beats missing from the workspace: {missing:?}",
                missing.len()
            );
        }
        let follows = collected_lines(&first, "follow.txt")?;
        if follows != [marker.clone()] {
            anyhow::bail!("{how}: follow.txt was {follows:?}");
        }
        println!(
            "PASS {how}: collected twice, same result; all {} journaled beats are in the workspace ({} written), and the follow-up",
            journaled.len(),
            in_file.len()
        );

        match client.call(&Request::Discard).await? {
            Response::Ack { outcome, .. } if outcome == "discarded" => {}
            other => anyhow::bail!("{how}: discard failed: {other:?}"),
        }
        let all = remote_text(
            &target,
            &["sudo", "-n", "docker", "ps", "-aq", "--filter", &label],
        )?;
        let volumes = remote_text(
            &target,
            &[
                "sudo", "-n", "docker", "volume", "ls", "-q", "--filter", &label,
            ],
        )?;
        if !all.trim().is_empty() || !volumes.trim().is_empty() {
            anyhow::bail!("{how}: discard left containers={all:?} volumes={volumes:?}");
        }
        match client.call(&Request::Collect).await? {
            Response::Err { message } if message == "no kept work" => {}
            other => anyhow::bail!("{how}: collect after discard returned {other:?}"),
        }
        println!("PASS {how}: discard removes the container and the volume, and nothing else does");
    }
    drop(tunnel);
    Ok(())
}

async fn export_in_container() -> anyhow::Result<()> {
    let target = ssh_target()?;
    // Git 2.30 is the bullseye package, not the host's Git.
    let output = remote_text(
        &target,
        &[
            "sudo",
            "-n",
            "docker",
            "run",
            "--rm",
            "-v",
            "/var/lib/brainiac-spike/src/scripts/export-git230.sh:/export.sh:ro",
            "debian:bullseye",
            "bash",
            "/export.sh",
        ],
    )?;
    print!("{output}");
    if !output.lines().any(|line| line == "PASS export") {
        anyhow::bail!("git 2.30 export did not pass");
    }
    Ok(())
}

async fn remote_script(name: &str) -> anyhow::Result<()> {
    let target = ssh_target()?;
    let path = format!("{STATE}/src/scripts/{name}");
    let output = remote_text(&target, &["sudo", "-n", "bash", &path])?;
    print!("{output}");
    if !output.lines().any(|line| line.starts_with("PASS")) {
        anyhow::bail!("{name} did not pass");
    }
    Ok(())
}

fn token(target: &str) -> anyhow::Result<String> {
    let text = remote_text(target, &["cat", &format!("{STATE}/token")])?;
    let token = text.trim().to_string();
    if token.is_empty() {
        anyhow::bail!("host token is empty");
    }
    Ok(token)
}

fn ssh_target() -> anyhow::Result<String> {
    let target =
        std::env::var("SPIKE_SSH").map_err(|_| anyhow::anyhow!("set SPIKE_SSH to the lab host"))?;
    if target.is_empty() || target.contains([' ', '\n', '\'']) {
        anyhow::bail!("SPIKE_SSH is empty or contains whitespace");
    }
    Ok(target)
}

fn remote(target: &str, args: &[&str]) -> anyhow::Result<()> {
    let output = ssh_output(target, args)?;
    if !output.status.success() {
        anyhow::bail!(
            "ssh {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn remote_text(target: &str, args: &[&str]) -> anyhow::Result<String> {
    let output = ssh_output(target, args)?;
    // systemctl is-active exits 3 when the unit is dead. The text is still useful.
    if !output.status.success() && output.stdout.is_empty() {
        anyhow::bail!(
            "ssh {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn ssh_output(target: &str, args: &[&str]) -> anyhow::Result<std::process::Output> {
    let mut command = Command::new("ssh");
    command.args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=8", target]);
    command.args(args);
    Ok(command.output()?)
}
