use crate::output::{Failure, Result, invalid};
use clap::{Arg, ArgAction, ArgMatches, Command};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
const RAW: &str = include_str!("../catalog/v1.json");
pub(crate) fn raw() -> &'static Value {
    static RAW_CATALOG: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    RAW_CATALOG.get_or_init(|| serde_json::from_str(RAW).expect("bundled catalog"))
}
#[derive(Deserialize)]
pub struct Catalog {
    pub version: String,
    #[serde(rename = "sourceVersion")]
    pub source_version: String,
    pub operations: Vec<Operation>,
    pub formats: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    pub operation_id: String,
    pub command: String,
    pub summary: String,
    pub effect: String,
    pub parameters: Vec<Parameter>,
    pub route: Option<String>,
    pub method: Option<String>,
    pub safe_probe: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}
#[derive(Deserialize)]
pub struct Parameter {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub required: bool,
    #[serde(default)]
    pub positional: bool,
    pub default: Option<Value>,
    pub minimum: Option<i64>,
    pub maximum: Option<i64>,
}
impl Catalog {
    pub(crate) fn discover(&self, query: &str, limit: usize) -> Value {
        let query = query.to_lowercase();
        let mut matches: Vec<_> = self
            .operations
            .iter()
            .filter_map(|op| {
                let rank = if query.is_empty()
                    || op.operation_id.to_lowercase() == query
                    || op.command.to_lowercase() == query
                {
                    0
                } else if op.tags.iter().any(|t| t.to_lowercase().contains(&query)) {
                    1
                } else if format!("{} {} {}", op.operation_id, op.command, op.summary)
                    .to_lowercase()
                    .contains(&query)
                {
                    2
                } else {
                    return None;
                };
                Some((rank, op))
            })
            .collect();
        matches.sort_by(|(a, x), (b, y)| a.cmp(b).then(x.operation_id.cmp(&y.operation_id)));
        let operations: Vec<_> = matches
            .into_iter()
            .take(limit)
            .map(|(_, op)| self.schema(&op.operation_id).unwrap())
            .collect();
        let suggestion = if operations.is_empty() {
            json!("使用tasks list查找待办、门店配置或点赞任务")
        } else {
            Value::Null
        };
        json!({"catalogVersion":self.version,"permissionGranted":false,"operations":operations,"suggestion":suggestion})
    }
    /// Immutable release catalog shared by all stages of one process.
    pub fn shared() -> &'static Self {
        static CATALOG: std::sync::OnceLock<Catalog> = std::sync::OnceLock::new();
        CATALOG.get_or_init(Self::load)
    }
    pub fn load() -> Self {
        serde_json::from_str(RAW).expect("bundled catalog")
    }
    pub fn digest(&self) -> String {
        format!("{:x}", Sha256::digest(RAW))
    }
    pub fn operation(&self, id: &str) -> Result<&Operation> {
        self.operations
            .iter()
            .find(|o| o.operation_id == id || o.command == id)
            .ok_or_else(|| Failure::new("UNKNOWN_OPERATION", 2, "操作未登记或尚未实现。"))
    }
    pub fn schema(&self, id: &str) -> Result<Value> {
        let op = self.operation(id)?;
        let raw = raw();
        let mut result = raw["operations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["operationId"] == op.operation_id)
            .unwrap()
            .clone();
        let mut properties = serde_json::Map::new();
        for parameter in &op.parameters {
            let mut schema = json!({"type":parameter.kind});
            if let Some(value) = &parameter.default {
                schema["default"] = value.clone();
            }
            if let Some(value) = parameter.minimum {
                schema["minimum"] = json!(value);
            }
            if let Some(value) = parameter.maximum {
                schema["maximum"] = json!(value);
            }
            properties.insert(parameter.name.clone(), schema);
        }
        properties.insert(
            "format".into(),
            json!({"type":"string","enum":self.formats,"default":self.formats[0]}),
        );
        if op.operation_id == "auth.login" {
            properties.insert(
                "login-method".into(),
                json!({"type":"string","enum":["password","dingtalk","sms"]}),
            );
        }
        result["inputSchema"] = json!({"type":"object","additionalProperties":false,
            "properties":properties,"required":op.parameters.iter().filter(|p|p.required).map(|p|&p.name).collect::<Vec<_>>()});
        Ok(result)
    }

    pub fn command(&self) -> Command {
        self.command_for_root(None)
    }

    /// Only construct the selected top-level command tree. Help/schema still use the full
    /// catalog, and the parser still owns all argument validation and global flags.
    pub(crate) fn invocation_command(&self, args: &[String]) -> Command {
        let root = args.iter().skip(1).find(|arg| {
            self.operations
                .iter()
                .any(|op| op.command.split(' ').next() == Some(arg.as_str()))
        });
        self.command_for_root(Some(root.map(String::as_str).unwrap_or("")))
    }

    fn command_for_root(&self, selected: Option<&str>) -> Command {
        fn base(name: String) -> Command {
            Command::new(name)
                .disable_help_flag(true)
                .disable_help_subcommand(true)
                .arg(
                    Arg::new("help")
                        .long("help")
                        .short('h')
                        .action(ArgAction::SetTrue),
                )
        }
        let mut root = base("dt-cli".into()).arg(
            Arg::new("format")
                .long("format")
                .global(true)
                .value_parser(clap::builder::PossibleValuesParser::new(
                    self.formats.clone(),
                ))
                .default_value(self.formats[0].clone()),
        );
        for op in &self.operations {
            let parts: Vec<_> = op.command.split(' ').collect();
            if selected.is_some_and(|root| parts[0] != root) {
                continue;
            }
            let mut cmd = base(parts.last().unwrap().to_string()).about(op.summary.clone());
            for p in &op.parameters {
                let mut arg = Arg::new(p.name.clone()).help(p.name.clone());
                if matches!(p.name.as_str(), "run" | "run-id" | "job-id" | "intent-id") {
                    // Opaque base64url IDs may legitimately begin with '-'. Executors still
                    // validate their exact alphabet and length before building any path or URL.
                    arg = arg.allow_hyphen_values(true);
                }
                if p.required {
                    arg = arg.required_unless_present("help");
                }
                if !p.positional {
                    arg = arg.long(p.name.clone());
                }
                if p.kind == "boolean" {
                    arg = arg.action(ArgAction::SetTrue);
                } else if let Some(d) = &p.default {
                    arg = arg.default_value(
                        d.as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| d.to_string()),
                    );
                }
                cmd = cmd.arg(arg);
            }
            if parts.len() == 1 {
                root = root.subcommand(cmd);
            } else {
                let group = parts[0].to_owned();
                if root.find_subcommand(&group).is_none() {
                    root = root.subcommand(base(group.clone()));
                }
                root = root.mut_subcommand(group, |g| g.subcommand(cmd));
            }
        }
        root
    }
    pub fn resolve<'a>(&'a self, m: &'a ArgMatches) -> Result<(&'a Operation, &'a ArgMatches)> {
        let Some((first, mut leaf)) = m.subcommand() else {
            return Ok((self.operation("help")?, m));
        };
        let mut path = first.to_owned();
        if let Some((second, nested)) = leaf.subcommand() {
            path = format!("{first} {second}");
            leaf = nested;
        }
        let op = self.operation(&path)?;
        if flag(m, "help") || flag(leaf, "help") {
            return Ok((op, leaf));
        }
        for p in &op.parameters {
            if p.kind == "integer" {
                let n = leaf
                    .get_one::<String>(&p.name)
                    .ok_or_else(invalid)?
                    .parse::<i64>()
                    .map_err(|_| invalid())?;
                if p.minimum.is_some_and(|v| n < v) || p.maximum.is_some_and(|v| n > v) {
                    return Err(invalid());
                }
            }
        }
        Ok((op, leaf))
    }
    pub fn help(&self) -> Value {
        json!({"catalogVersion":self.version,"commands":self.operations.iter().map(|o|json!({"operationId":o.operation_id,"command":o.command,"summary":o.summary})).collect::<Vec<_>>()})
    }
}
pub fn string<'a>(m: &'a ArgMatches, key: &str) -> Option<&'a str> {
    m.try_get_one::<String>(key)
        .ok()
        .flatten()
        .map(String::as_str)
}
pub fn flag(m: &ArgMatches, key: &str) -> bool {
    m.try_get_one::<bool>(key)
        .ok()
        .flatten()
        .copied()
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_command_tree_preserves_all_catalog_arguments() {
        let catalog = Catalog::shared();
        for operation in &catalog.operations {
            for help in [false, true] {
                let mut args = vec!["dt-cli".to_owned(), "--format".into(), "table".into()];
                args.extend(operation.command.split(' ').map(str::to_owned));
                if help {
                    args.push("--help".into());
                } else {
                    for parameter in operation.parameters.iter().filter(|p| p.required) {
                        if !parameter.positional {
                            args.push(format!("--{}", parameter.name));
                        }
                        if parameter.kind != "boolean" {
                            args.push(if parameter.kind == "integer" {
                                parameter.minimum.unwrap_or(1).to_string()
                            } else {
                                "fixture".into()
                            });
                        }
                    }
                }
                let full = catalog.command().try_get_matches_from(&args).unwrap();
                let selected = catalog
                    .invocation_command(&args)
                    .try_get_matches_from(&args)
                    .unwrap();
                let (full_op, full_leaf) = catalog.resolve(&full).unwrap();
                let (selected_op, selected_leaf) = catalog.resolve(&selected).unwrap();
                assert_eq!(full_op.operation_id, selected_op.operation_id, "{args:?}");
                assert_eq!(string(&full, "format"), string(&selected, "format"));
                for parameter in &operation.parameters {
                    assert_eq!(
                        string(full_leaf, &parameter.name),
                        string(selected_leaf, &parameter.name),
                        "{args:?}"
                    );
                    assert_eq!(
                        flag(full_leaf, &parameter.name),
                        flag(selected_leaf, &parameter.name),
                        "{args:?}"
                    );
                }
            }
        }
        for args in [
            vec!["dt-cli"],
            vec!["dt-cli", "--help"],
            vec!["dt-cli", "auth", "--help"],
            vec!["dt-cli", "--format", "bad", "version"],
            vec!["dt-cli", "bogus", "version"],
            vec!["dt-cli", "auth", "bogus"],
            vec!["dt-cli", "--unknown", "version"],
            vec!["dt-cli", "discover", "--query", "auth", "--format", "json"],
            vec![
                "dt-cli", "discover", "--query", "auth", "--query", "version",
            ],
        ] {
            let args: Vec<String> = args.into_iter().map(str::to_owned).collect();
            assert_eq!(
                catalog.command().try_get_matches_from(&args).is_ok(),
                catalog
                    .invocation_command(&args)
                    .try_get_matches_from(&args)
                    .is_ok(),
                "{args:?}"
            );
        }
    }
}
