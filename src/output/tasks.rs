//! Compact human task output. JSON remains the complete machine-readable contract.
use serde_json::Value;
use std::fmt::Write;
fn text(v: &Value) -> String {
    if v.is_null() {
        return "未知".into();
    }
    let s = if let Some(s) = v.as_str() {
        s.to_owned()
    } else {
        v.to_string()
    };
    s.chars()
        .flat_map(|c| {
            if c.is_control() || matches!(c,'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}') {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}
fn row(out: &mut String, label: &str, value: &Value) {
    let _ = writeln!(out, "{label}\t{}", text(value));
}
pub(super) fn render(v: &Value) -> Option<String> {
    if !v["operationId"].as_str()?.starts_with("tasks.") {
        return None;
    }
    let mut out = String::new();
    if v["ok"] == false {
        row(&mut out, "错误", &v["error"]["code"]);
        row(&mut out, "说明", &v["error"]["message"]);
    }
    let data = &v["data"];
    if let Some(tasks) = data["tasks"].as_array() {
        out.push_str("任务\t名称\t可用状态\n");
        for task in tasks {
            let _ = writeln!(
                out,
                "{}\t{}\t{}",
                text(&task["taskId"]),
                text(&task["title"]),
                text(&task["availability"]["state"])
            );
        }
        if tasks.is_empty() {
            row(&mut out, "建议", &data["suggestion"]);
        }
    } else {
        if !data["taskId"].is_null() {
            row(&mut out, "任务", &data["taskId"]);
        }
        if !data["runId"].is_null() {
            row(&mut out, "执行记录", &data["runId"]);
        }
        let result = &data["result"];
        if result["preview"] == true {
            out.push_str("预览：未执行\n");
            row(&mut out, "绑定", &result["binding"]);
            row(&mut out, "步骤", &result["steps"]);
            row(&mut out, "上限", &result["limits"]);
        } else if data["taskId"] == "inbox.brief" {
            row(&mut out, "计数", &result["counts"]);
            out.push_str("ID\t类别\t应用\t标题\t发起人\t到达时间\n");
            if let Some(items) = result["items"].as_array() {
                for item in items {
                    let _ = writeln!(
                        out,
                        "{}\t{}\t{}\t{}\t{}\t{}",
                        text(&item["id"]),
                        text(&item["kind"]),
                        text(&item["applicationName"]),
                        text(&item["title"]),
                        text(&item["initiatorName"]),
                        text(&item["arrivedAt"])
                    );
                    for key in [
                        "applicationId",
                        "formType",
                        "startedAt",
                        "processedAt",
                        "endedAt",
                    ] {
                        row(&mut out, key, &item[key]);
                    }
                    if !item["openAction"].is_null() {
                        row(&mut out, "选中后打开", &item["openAction"]);
                    }
                }
            }
            for key in ["filters", "groups", "sort", "limitations"] {
                if !result[key].is_null() {
                    row(&mut out, key, &result[key]);
                }
            }
        } else if data["taskId"] == "order-config.compare" && result["items"].is_array() {
            out.push_str("品项\t字段\t比较\t门店源值\n");
            for item in result["items"].as_array().unwrap() {
                for field in item["fields"].as_array().into_iter().flatten() {
                    let source_values:Vec<_>=item["sources"].as_array().into_iter().flatten().map(|source|serde_json::json!({"shopCode":source["shopCode"],"state":source["state"],"value":source["values"][field["field"].as_str().unwrap_or("")]})).collect();
                    let _ = writeln!(
                        out,
                        "{}\t{}\t{}\t{}",
                        text(&item["itemCode"]),
                        text(&field["field"]),
                        text(&field["state"]),
                        text(&serde_json::json!(source_values))
                    );
                }
            }
            row(&mut out, "无记录门店", &result["shopsWithoutRows"]);
            out.push_str("来源值比较；各门店读取不构成同一时点快照。\n");
        } else if result.is_object() {
            for key in [
                "jobId",
                "intentId",
                "state",
                "sideEffect",
                "complete",
                "resultRecovery",
            ] {
                if !result[key].is_null() {
                    row(&mut out, key, &result[key]);
                }
            }
        } else if data.is_object() {
            for key in ["title", "description", "availability", "inputSchema"] {
                if !data[key].is_null() {
                    row(&mut out, key, &data[key]);
                }
            }
        }
    }
    if let Some(complete) = v["meta"]["complete"].as_bool() {
        let _ = writeln!(
            out,
            "完整性\t{}",
            if complete {
                "完整"
            } else {
                "未完成 / 部分结果"
            }
        );
    }
    for key in ["availability", "observedAt", "sources", "recordPersistence"] {
        if !v["meta"][key].is_null() {
            row(&mut out, key, &v["meta"][key]);
        }
    }
    if let Some(actions) = v["meta"]["actions"].as_array() {
        for (index, action) in actions.iter().enumerate() {
            let _ = writeln!(
                out,
                "{}动作\t{}\t{}",
                if index == 0 { "主要恢复" } else { "后续" },
                text(&action["actor"]),
                text(&action["reason"])
            );
            for key in ["argv", "url"] {
                if !action[key].is_null() {
                    row(&mut out, key, &action[key]);
                }
            }
        }
    }
    Some(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn partial_task_and_untrusted_control_text_remain_clear() {
        let v = json!({"operationId":"tasks.run","ok":false,"error":{"code":"PARTIAL_RESULT","message":"部分结果"},"data":{"taskId":"inbox.brief","runId":null,"result":{"counts":{"todo":1,"cc":null},"items":[{"kind":"todo","source":"x","title":"恶意\u{1b}[2J\n标题","status":null,"arrivalAt":null}]}},"meta":{"complete":false,"actions":[{"actor":"employee","reason":"查询","argv":["dt-cli","tasks","status","id"]}]}});
        let out = render(&v).unwrap();
        assert!(!out.contains('\u{1b}'));
        assert!(out.contains("\\u{1b}"));
        assert!(out.contains("cc"));
        assert!(out.contains("未完成"));
        assert!(out.contains("主要恢复动作"));
    }
}
