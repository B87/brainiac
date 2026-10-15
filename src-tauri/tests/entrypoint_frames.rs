//! The run image's entrypoint reads one credential frame from stdin
//! (docs/architecture.md, Agent runs — v0.5, Credentials). This feeds it the
//! frames the controller writes and the ones it must refuse, and reads the
//! reason it prints. A frame it accepts goes on to look for the run's start,
//! which is not here, so it stops with "clone": that is how acceptance shows.
//! It needs `node`; without it the test says so and passes.

use std::io::Write;
use std::process::{Command, Stdio};

use brainiac_lib::agents::controller::protocol::{Credential, CredentialKey, ExtraSecret};
use brainiac_lib::agents::controller::{frame, frame_with};

const ENTRYPOINT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/agents/image/entrypoint.mjs"
);

fn credential() -> Credential {
    Credential {
        key: CredentialKey::AnthropicApiKey,
        value: "sk-ant-api03-example-key-value".into(),
    }
}

fn extra(name: &str, value: &str) -> ExtraSecret {
    ExtraSecret {
        name: name.into(),
        value: value.into(),
    }
}

/// What the entrypoint says for these bytes on stdin, or `None` without node.
fn reply(bytes: &[u8]) -> Option<String> {
    let mut child = Command::new("node")
        .arg(ENTRYPOINT)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(bytes).ok()?;
    let out = child.wait_with_output().ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn raw(magic: &[u8; 4], json: &str) -> Vec<u8> {
    let mut frame = magic.to_vec();
    frame.extend_from_slice(&(json.len() as u32).to_be_bytes());
    frame.extend_from_slice(json.as_bytes());
    frame
}

#[test]
fn the_entrypoint_accepts_the_controllers_frames_and_refuses_the_rest() {
    let Some(plain) = reply(&frame(credential())) else {
        eprintln!("node is not installed; skipping");
        return;
    };
    assert_eq!(plain, "bootstrap:rejected clone", "BRB1 is accepted");

    let with_extra = frame_with(
        credential(),
        vec![
            extra("SENTRY_TOKEN", "sntrys_example_token"),
            extra("GCP_KEY_JSON", "{\n \"type\": \"service_account\"\n}"),
        ],
    );
    assert_eq!(
        reply(&with_extra).unwrap(),
        "bootstrap:rejected clone",
        "BRB2 is accepted"
    );

    let cred = r#"{"ANTHROPIC_API_KEY":"sk-ant-api03-example-key-value"}"#;
    for (what, bytes, reason) in [
        ("an unknown magic", raw(b"BRB3", cred), "magic"),
        (
            "extras under BRB1",
            raw(
                b"BRB1",
                r#"{"ANTHROPIC_API_KEY":"sk-ant-api03-example-key-value","SENTRY_TOKEN":"12345678"}"#,
            ),
            "keys",
        ),
        (
            "a name that loads code",
            frame_with(credential(), vec![extra("NODE_OPTIONS", "--require=/x")]),
            "extra",
        ),
        (
            "a name an agent reads",
            frame_with(
                credential(),
                vec![extra("ANTHROPIC_BASE_URL", "http://x.example")],
            ),
            "extra",
        ),
        (
            "a proxy",
            frame_with(credential(), vec![extra("HTTPS_PROXY", "http://x.example")]),
            "extra",
        ),
        (
            "a short value",
            frame_with(credential(), vec![extra("SENTRY_TOKEN", "short")]),
            "extra",
        ),
        (
            "no extras in a BRB2 frame",
            raw(b"BRB2", &format!(r#"{{"credential":{cred},"extra":{{}}}}"#)),
            "extra",
        ),
        (
            "a second credential",
            raw(
                b"BRB2",
                r#"{"credential":{"ANTHROPIC_API_KEY":"sk-ant-api03-example-key-value","OPENAI_API_KEY":"sk-example-example-1234"},"extra":{"SENTRY_TOKEN":"12345678"}}"#,
            ),
            "keys",
        ),
        (
            "a stray top-level key",
            raw(
                b"BRB2",
                &format!(
                    r#"{{"credential":{cred},"extra":{{"SENTRY_TOKEN":"12345678"}},"more":1}}"#
                ),
            ),
            "keys",
        ),
    ] {
        assert_eq!(
            reply(&bytes).unwrap(),
            format!("bootstrap:rejected {reason}"),
            "{what}"
        );
    }
}
