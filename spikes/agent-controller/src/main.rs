//! Spike commands. This binary is not the Brainiac app.

mod acp;
mod controller;
mod engine;
mod guard;
mod journal;
mod ledger;
mod local;
mod messages;
mod prove;
mod state;

use state::{state_from_args, StateDir};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let command = std::env::args().nth(1);
    match command.as_deref() {
        Some("controller") => controller::serve(StateDir::new(state_from_args())).await,
        Some("guard") => guard::serve(StateDir::new(state_from_args())).await,
        Some("emergency-stop") => guard::emergency_stop(StateDir::new(state_from_args())).await,
        Some("prove") => {
            let section = std::env::args().nth(2);
            if section.as_deref() == Some("local") {
                local::run().await
            } else {
                prove::run(section.as_deref()).await
            }
        }
        Some("hold") => match std::env::args().nth(2).as_deref() {
            Some("claude") => prove::hold_claude().await,
            Some("local") => local::lid().await,
            None => prove::hold().await,
            Some(other) => anyhow::bail!("unknown hold section {other}"),
        },
        Some("-h") | Some("--help") | None => {
            print_usage();
            Ok(())
        }
        Some(other) => anyhow::bail!("unknown command {other}\n{}", usage()),
    }
}

fn print_usage() {
    println!("{}", usage());
}

fn usage() -> String {
    format!(
        "\
brainiac-spike — host-owned controller proof

  controller --state {state}     own the agent attach (runs on the host)
  guard --state {state}          stop containers if the controller dies
  emergency-stop --state {state} authenticated stop on the host
  prove [session|limit|export|quota|collect|interrupt|claude|local]
                                 run checks; SPIKE_SSH selects the host
                                 local runs the controller on this Mac
                                 claude also needs SPIKE_CLAUDE_TOKEN
  hold [claude|local]            sleep or close the lid while a session runs
                                 claude also needs SPIKE_CLAUDE_TOKEN

Deploy with scripts/deploy.sh. Do not put the host name in the repository.
",
        state = state::DEFAULT_STATE
    )
}
