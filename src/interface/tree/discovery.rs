use clap::{Arg, Command};
pub(super) fn target(command: Command) -> Command {
    command
        .arg(
            Arg::new("operation")
                .long("operation")
                .required_unless_present("alias"),
        )
        .arg(Arg::new("alias").long("alias").conflicts_with("operation"))
        .arg(Arg::new("system").long("system"))
        .arg(Arg::new("version").long("version"))
}
pub(super) fn commands() -> Vec<Command> {
    let profile = || Arg::new("profile").long("profile");
    vec![
        Command::new("search")
            .arg(profile())
            .arg(Arg::new("system").long("system"))
            .arg(Arg::new("name").long("name"))
            .arg(Arg::new("method").long("method"))
            .arg(Arg::new("path").long("path"))
            .arg(Arg::new("page").long("page").default_value("1"))
            .arg(Arg::new("page-size").long("page-size").default_value("20")),
        target(Command::new("resolve").arg(profile())),
        target(Command::new("describe").arg(profile())),
        target(Command::new("template").arg(profile()))
            .arg(Arg::new("output").long("output"))
            .arg(
                Arg::new("overwrite-digest")
                    .long("overwrite-digest")
                    .requires("output"),
            ),
    ]
}
