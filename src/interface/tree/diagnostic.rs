use clap::{Arg, Command};
pub(super) fn command() -> Command {
    super::target(Command::new("diagnose").arg(Arg::new("profile").long("profile")))
        .arg(
            Arg::new("probe")
                .long("probe")
                .action(clap::ArgAction::SetTrue)
                .requires_all(["input-file", "scope"]),
        )
        .arg(Arg::new("input-file").long("input-file").requires("probe"))
        .arg(
            Arg::new("body-file")
                .long("body-file")
                .requires("input-file"),
        )
        .arg(Arg::new("scope").long("scope").requires("probe"))
}
