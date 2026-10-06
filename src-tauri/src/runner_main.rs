//! `brainiac-runner`: the run controller and its guard, without the GUI or
//! the Keychain. Deploy installs this binary on a Linux host
//! (docs/architecture.md, Remote hosts). On this Mac the app executable
//! is still `brainiac runner …`.

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // A guard started by a build that still passes the `runner` word.
    if args.first().map(String::as_str) == Some("runner") {
        args.remove(0);
    }
    std::process::exit(brainiac_lib::agents::controller::main(&args));
}
