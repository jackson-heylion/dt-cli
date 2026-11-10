use clap::{ArgAction, Command};
use serde::Serialize;
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Parameter {
    /// 参数名，帮助中不含实际输入。
    name: String,
    /// 是否为必须项；组合约束见条件字段。
    required: bool,
    /// 本命令声明的默认值。
    defaults: Vec<String>,
    /// 布尔开关或具有值的参数。
    kind: &'static str,
    /// 不能同时提供的参数。
    conflicts: Vec<String>,
}
#[derive(Serialize)]
pub(crate) struct Entry {
    /// 从唯一命令树得到的可用动作。
    pub(crate) command: String,
    /// 该动作直接接受的参数。
    parameters: Vec<Parameter>,
    /// Clap 根据必填项、组合与互斥条件生成的用法。
    usage: String,
    /// 参数说明与可选值来自同一命令定义。
    help: String,
}
pub(crate) fn entries(node: &Command, path: &str, output: &mut Vec<Entry>) {
    let parameters = node
        .get_arguments()
        .filter(|a| !a.is_hide_set())
        .map(|a| Parameter {
            name: format!("--{}", a.get_long().unwrap_or(a.get_id().as_str())),
            required: a.is_required_set(),
            defaults: a
                .get_default_values()
                .iter()
                .map(|v| v.to_string_lossy().into_owned())
                .collect(),
            kind: if matches!(a.get_action(), ArgAction::SetTrue) {
                "flag"
            } else {
                "value"
            },
            conflicts: node
                .get_arg_conflicts_with(a)
                .into_iter()
                .map(|v| format!("--{}", v.get_long().unwrap_or(v.get_id().as_str())))
                .collect(),
        })
        .collect();
    let mut view = node.clone().bin_name(path);
    output.push(Entry {
        command: path.to_owned(),
        parameters,
        usage: view.render_usage().to_string(),
        help: view.render_long_help().to_string(),
    });
    for child in node.get_subcommands() {
        entries(child, &format!("{path} {}", child.get_name()), output);
    }
}
