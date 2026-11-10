use clap::{Arg, Command};
pub(super) fn command() -> Command {
    let choice = || {
        Arg::new("purpose")
            .long("purpose")
            .default_value("business")
            .value_parser(["business", "management"])
    };
    Command::new("context")
        .subcommand(
            Command::new("use")
                .arg(Arg::new("profile").long("profile").required(true))
                .arg(Arg::new("environment").long("environment").required(true))
                .arg(choice()),
        )
        .subcommand(Command::new("show").arg(choice()))
        .subcommand(Command::new("clear").arg(choice()))
}
