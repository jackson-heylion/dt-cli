//! 唯一公开命令定义、帮助及安全解析边界。
use crate::{
    Runtime,
    output::{self, Result, invalid},
};
use clap::{ArgMatches, Command};
use serde_json::Value;
mod parse;
pub(crate) mod request;

mod tree;
use tree::command;
fn help(root: &Command, parsed: &ArgMatches) -> Result<Value> {
    let mut scope = "dt-cli".to_owned();
    let mut node = root;
    let mut leaf = parsed;
    while let Some((name, next)) = leaf.subcommand() {
        if name == "help" {
            break;
        }
        let Some(child) = node.find_subcommand(name) else {
            break;
        };
        scope.push(' ');
        scope.push_str(name);
        node = child;
        leaf = next;
    }
    if scope == "dt-cli" {
        let mut value = tree::definition();
        value["scope"] = serde_json::json!(scope);
        return Ok(value);
    }
    let mut commands = vec![];
    tree::entries(node, &scope, &mut commands);
    Ok(serde_json::json!({"scope":scope,"commands":commands}))
}

pub(crate) fn string<'a>(m: &'a ArgMatches, key: &str) -> Option<&'a str> {
    m.try_get_one::<String>(key)
        .ok()
        .flatten()
        .map(String::as_str)
}
pub(crate) fn flag(m: &ArgMatches, key: &str) -> bool {
    m.try_get_one::<bool>(key)
        .ok()
        .flatten()
        .copied()
        .unwrap_or(false)
}

pub(crate) async fn execute(rt: &Runtime, args: Vec<String>) -> (Value, u8, bool) {
    let root = command();
    let wants_help = args.iter().any(|v| v == "--help" || v == "-h");
    let diagnostic_args = args.clone();
    let parsed = match root
        .clone()
        .ignore_errors(wants_help)
        .try_get_matches_from(args)
    {
        Ok(v) => v,
        Err(error) => {
            let mut failure = invalid();
            let kind = match error.kind() {
                clap::error::ErrorKind::MissingRequiredArgument => "missing_required_argument",
                clap::error::ErrorKind::ArgumentConflict => "conflicting_arguments",
                clap::error::ErrorKind::UnknownArgument => "unknown_argument",
                clap::error::ErrorKind::InvalidSubcommand => "unknown_command",
                clap::error::ErrorKind::InvalidValue => "invalid_value",
                _ => "invalid_argument",
            };
            let parsed = root
                .clone()
                .ignore_errors(true)
                .try_get_matches_from(diagnostic_args)
                .ok();
            failure.details = Some(crate::output::Details {
                argument_kind: Some(kind.into()),
                scope: Some(
                    parsed
                        .as_ref()
                        .map(scope)
                        .unwrap_or_else(|| "dt-cli".into()),
                ),
                ..Default::default()
            });
            let (v, c) = output::envelope_result("cli", None, Err(failure));
            return (v, c, false);
        }
    };
    let table = string(&parsed, "format") == Some("table");
    let mut leaf = &parsed;
    while let Some((_, next)) = leaf.subcommand() {
        leaf = next;
    }
    if wants_help
        || parsed.subcommand_name().is_none()
        || parsed.subcommand_name() == Some("help")
        || ["context", "jobs", "writes", "manage", "skill"].contains(&leaf_id(&parsed))
            && parsed.subcommand().unwrap().1.subcommand().is_none()
    {
        let (v, c) = output::envelope_result("help", None, help(&root, &parsed));
        return (v, c, table);
    }
    let (name, options) = parsed.subcommand().unwrap();
    let operation = match name {
        "version" | "install" | "upgrade" | "doctor" => name.to_owned(),
        "skill" => "skill.install".to_owned(),
        _ => format!("platform.{name}"),
    };
    let result = match name {
        "version" => Ok(crate::release::build_info()),
        "install" | "upgrade" => crate::release::execute(name, &parse::native(name, options)).await,
        "skill" => match parse::skill(options.subcommand().unwrap().1) {
            Ok(input) => crate::release::install_skill(&input).await,
            Err(e) => Err(e),
        },
        "doctor" => doctor(rt, options),
        _ => match parse::platform(&parsed) {
            Ok(input) => crate::platform::execute(rt, input).await,
            Err(e) => Err(e),
        },
    };
    let (v, c) = output::envelope_result(&operation, None, result);
    (v, c, table)
}
fn leaf_id(parsed: &ArgMatches) -> &str {
    parsed.subcommand_name().unwrap_or("")
}
fn doctor(rt: &Runtime, options: &ArgMatches) -> Result<Value> {
    if flag(options, "workbuddy") {
        return crate::workbuddy::doctor(&rt.root, flag(options, "fix"));
    }
    if flag(options, "storage") {
        let credentials = rt.store.preflight()?;
        let platform = crate::private_store::probe_metadata(&rt.root.join("platform"))?;
        Ok(
            serde_json::json!({"onlineVerified":false,"credentials":credentials,"platform":platform}),
        )
    } else {
        Ok(
            serde_json::json!({"onlineVerified":false,"cliVersion":env!("CARGO_PKG_VERSION"),"credentialStore":"not-probed"}),
        )
    }
}

fn scope(matches: &ArgMatches) -> String {
    let mut name = "dt-cli".to_owned();
    let mut current = matches;
    while let Some((command, next)) = current.subcommand() {
        name.push(' ');
        name.push_str(command);
        current = next;
    }
    name
}
