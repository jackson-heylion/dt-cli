//! 同一棵无业务依赖的命令树，运行时与构建事实共用。
use clap::{Arg, ArgAction, Command};
#[path = "tree/context.rs"]
mod context;
#[path = "tree/diagnostic.rs"]
mod diagnostic;
#[path = "tree/discovery.rs"]
mod discovery;
#[path = "tree/management.rs"]
mod management;
#[path = "tree/platform.rs"]
mod platform;
use discovery::target;
#[path = "tree/facts.rs"]
mod facts;
pub(super) use facts::entries;
fn switch(name: &str) -> Arg {
    Arg::new(name.to_owned())
        .long(name.to_owned())
        .action(ArgAction::SetTrue)
}
fn native(name: &str) -> Command {
    let mut command = Command::new(name.to_owned());
    for key in [
        "package",
        "directory",
        "sha256",
        "signer",
        "minimum-version",
    ] {
        command = command.arg(Arg::new(key).long(key));
    }
    for key in [
        "check",
        "rollback",
        "prune",
        "candidate",
        "online",
        "cached",
    ] {
        command = command.arg(switch(key));
    }
    command
}

pub(crate) fn command() -> Command {
    let mut command = normalize(
        platform::platform()
            .arg(switch("help").short('h').global(true))
            .arg(
                Arg::new("format")
                    .long("format")
                    .default_value("json")
                    .value_parser(["json", "table"])
                    .global(true),
            )
            .subcommand(Command::new("help"))
            .subcommand(Command::new("version"))
            .subcommand(native("install"))
            .subcommand(native("upgrade"))
            .subcommand(
                Command::new("skill").subcommand(
                    Command::new("install")
                        .arg(Arg::new("directory").long("directory").required(true))
                        .arg(Arg::new("name").long("name").default_value("dt-cli"))
                        .arg(switch("check"))
                        .arg(switch("rollback")),
                ),
            )
            .subcommand(
                Command::new("doctor")
                    .arg(switch("storage").conflicts_with("workbuddy"))
                    .arg(switch("workbuddy"))
                    .arg(switch("fix").requires("workbuddy")),
            ),
    );
    command.build();
    command
}

fn normalize(command: Command) -> Command {
    command
        .disable_help_flag(true)
        .disable_help_subcommand(true)
        .mut_subcommands(normalize)
}

pub(crate) fn definition() -> serde_json::Value {
    let command = command();
    let mut definitions = vec![];
    entries(&command, "dt-cli", &mut definitions);
    serde_json::json!({"version":"platform-root-v1","sourceVersion":"platform-root-v1","commands":definitions})
}
