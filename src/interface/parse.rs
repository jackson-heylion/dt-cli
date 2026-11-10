use super::{flag, request::*, string};
use crate::output::{Result, invalid};
use clap::ArgMatches;
fn owned(m: &ArgMatches, key: &str) -> Option<String> {
    string(m, key).map(str::to_owned)
}
fn text(m: &ArgMatches, key: &str) -> String {
    owned(m, key).unwrap_or_default()
}
fn number(m: &ArgMatches, key: &str, default: u32) -> Result<u32> {
    string(m, key)
        .map(|v| v.parse().map_err(|_| invalid()))
        .unwrap_or(Ok(default))
}
fn page(m: &ArgMatches) -> Result<u32> {
    let page = number(m, "page", 1)?;
    if !(1..=100000).contains(&page) {
        return Err(invalid());
    }
    Ok(page)
}
fn target(m: &ArgMatches) -> Result<Target> {
    let version = string(m, "version")
        .map(|v| v.parse::<u64>().map_err(|_| invalid()))
        .transpose()?;
    if version == Some(0) {
        return Err(invalid());
    }
    Ok(Target {
        operation: text(m, "operation"),
        alias: text(m, "alias"),
        system: text(m, "system"),
        version,
    })
}
fn files(m: &ArgMatches) -> Files {
    Files {
        input: text(m, "input-file"),
        body: owned(m, "body-file"),
    }
}
fn login(m: &ArgMatches) -> Login {
    Login {
        environment: text(m, "environment"),
        origin: owned(m, "origin"),
        system: string(m, "system").unwrap_or("platform-management").into(),
        no_browser: flag(m, "no-browser"),
    }
}
fn record(m: &ArgMatches) -> Result<Record> {
    let (name, m) = m.subcommand().ok_or_else(invalid)?;
    let action = match name {
        "list" => "list",
        "start" => "start",
        "resume" => "resume",
        "status" => "status",
        "results" => "results",
        "prepare" => "prepare",
        "inspect" => "inspect",
        "confirm" => "confirm",
        "dispatch" => "dispatch",
        _ => return Err(invalid()),
    };
    let page = if action == "results" {
        number(m, "page", 0)?
    } else {
        page(m)?
    };
    Ok(Record {
        action,
        profile: owned(m, "profile"),
        id: text(m, "id"),
        page,
        target: target(m)?,
        files: files(m),
        instruction: text(m, "instruction"),
        fingerprint: text(m, "fingerprint"),
    })
}
pub(super) fn platform(m: &ArgMatches) -> Result<Platform> {
    let (name, m) = m.subcommand().ok_or_else(invalid)?;
    let action = match name {
        "profiles" => Action::Profiles,
        "login" => Action::Login(login(m)),
        "check" => Action::Check,
        "sessions" => Action::Sessions(page(m)?),
        "revoke" => Action::Revoke(text(m, "session")),
        "logout" => Action::Logout,
        "catalog" => Action::Catalog(page(m)?),
        "views" => Action::Views(target(m)?),
        "call" => Action::Call(Call {
            target: target(m)?,
            files: files(m),
            path: text(m, "path"),
            query: text(m, "query"),
            view: owned(m, "view")
                .map(|v| v.parse().map_err(|_| invalid()))
                .transpose()?,
            object: owned(m, "object"),
        }),
        "search" => {
            let size = number(m, "page-size", 20)?;
            if !(1..=100).contains(&size) {
                return Err(invalid());
            }
            Action::Search(Search {
                system: text(m, "system"),
                name: text(m, "name"),
                method: text(m, "method"),
                path: text(m, "path"),
                page: page(m)?,
                page_size: size,
            })
        }
        "resolve" => Action::Resolve(target(m)?),
        "describe" => Action::Describe(target(m)?),
        "template" => Action::Template(
            target(m)?,
            Template {
                output: owned(m, "output"),
                overwrite_digest: owned(m, "overwrite-digest"),
            },
        ),
        "diagnose" => Action::Diagnose(Diagnostic {
            target: target(m)?,
            probe: flag(m, "probe"),
            files: files(m),
            scope: text(m, "scope"),
        }),
        "jobs" => Action::Jobs(record(m)?),
        "writes" => Action::Writes(record(m)?),
        "context" => {
            let (name, m) = m.subcommand().ok_or_else(invalid)?;
            let action = match name {
                "use" => "use",
                "show" => "show",
                "clear" => "clear",
                _ => return Err(invalid()),
            };
            let purpose = match string(m, "purpose") {
                Some("management") => "management",
                _ => "business",
            };
            return Ok(Platform {
                profile: None,
                action: Action::Context(Context {
                    action,
                    profile: owned(m, "profile"),
                    environment: text(m, "environment"),
                    purpose,
                }),
            });
        }
        "manage" => {
            let (name, m) = m.subcommand().ok_or_else(invalid)?;
            let action = match name {
                "login" => "login",
                "check" => "check",
                "logout" => "logout",
                "capabilities" => "capabilities",
                "systems" => "systems",
                "drafts" => "drafts",
                "versions" => "versions",
                "grants" => "grants",
                "audit" => "audit",
                "plan" => "plan",
                "inspect" => "inspect",
                "status" => "status",
                "apply" => "apply",
                _ => return Err(invalid()),
            };
            let request = Management {
                action,
                profile: owned(m, "profile"),
                login: if action == "login" {
                    Some(login(m))
                } else {
                    None
                },
                id: owned(m, "id"),
                page: if string(m, "page").is_some() {
                    Some(page(m)?)
                } else {
                    None
                },
                system: owned(m, "system"),
                manifest: text(m, "manifest"),
                digest: text(m, "digest"),
                request_id: owned(m, "request-id"),
                retry: flag(m, "retry"),
            };
            return Ok(Platform {
                profile: None,
                action: Action::Manage(request),
            });
        }
        _ => return Err(invalid()),
    };
    Ok(Platform {
        profile: owned(m, "profile"),
        action,
    })
}
pub(super) fn native(operation: &str, m: &ArgMatches) -> Native {
    Native {
        install: operation == "install",
        check: flag(m, "check"),
        rollback: flag(m, "rollback"),
        prune: flag(m, "prune"),
        candidate: flag(m, "candidate"),
        online: flag(m, "online"),
        cached: flag(m, "cached"),
        package: owned(m, "package"),
        directory: owned(m, "directory"),
        sha256: owned(m, "sha256"),
        signer: owned(m, "signer"),
        minimum_version: owned(m, "minimum-version"),
    }
}
pub(super) fn skill(m: &ArgMatches) -> Result<Skill> {
    Ok(Skill {
        name: text(m, "name"),
        directory: owned(m, "directory").ok_or_else(invalid)?,
        check: flag(m, "check"),
        rollback: flag(m, "rollback"),
    })
}
