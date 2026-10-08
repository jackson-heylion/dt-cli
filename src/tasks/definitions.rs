use crate::output::{Failure, Result, invalid};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Definition {
    pub task_schema_version: u32,
    pub recipe_version: u32,
    pub task_id: String,
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
    pub provider: String,
    pub system_id: Option<String>,
    pub effect: String,
    pub operation_id: String,
    pub input_schema: Value,
    pub required_output_fields: Vec<String>,
}

pub(super) fn all() -> &'static [Definition] {
    static DEFINITIONS: OnceLock<Vec<Definition>> = OnceLock::new();
    DEFINITIONS.get_or_init(|| {
        serde_json::from_str(include_str!("../../catalog/tasks.json"))
            .expect("reviewed task definitions")
    })
}
pub(super) fn get(id: &str) -> Result<&'static Definition> {
    all()
        .iter()
        .find(|d| d.task_id == id)
        .ok_or_else(|| Failure::new("UNKNOWN_TASK", 2, "任务未登记；请用tasks list查找。"))
}
impl Definition {
    pub(super) fn view(&self) -> Value {
        json!({"taskSchemaVersion":self.task_schema_version,"recipeVersion":self.recipe_version,"taskId":self.task_id,
            "title":self.title,"description":self.description,"effect":self.effect,"tags":self.tags,
            "provider":self.provider,"systemId":self.system_id,"requiredOperation":self.operation_id,"inputSchema":self.input_schema})
    }
    pub(super) fn rank(&self, query: &str) -> Option<u8> {
        if query.is_empty() || self.task_id.to_lowercase() == query {
            Some(0)
        } else if self
            .tags
            .iter()
            .any(|tag| tag.to_lowercase().contains(query))
        {
            Some(1)
        } else if format!("{} {}", self.title, self.description)
            .to_lowercase()
            .contains(query)
        {
            Some(2)
        } else {
            None
        }
    }
    pub(super) fn validate(&self, raw: &str) -> Result<Value> {
        static VALIDATORS: OnceLock<BTreeMap<String, jsonschema::Validator>> = OnceLock::new();
        let validators = VALIDATORS.get_or_init(|| {
            all()
                .iter()
                .map(|d| {
                    (
                        d.task_id.clone(),
                        jsonschema::validator_for(&d.input_schema).expect("reviewed task schema"),
                    )
                })
                .collect()
        });
        let value = crate::input::strict_json(raw)?;
        if !validators[&self.task_id].is_valid(&value) {
            return Err(invalid());
        }
        let mut dates = Vec::new();
        for key in ["startedFrom", "startedTo"] {
            if let Some(text) = value.get(key).and_then(Value::as_str) {
                let date =
                    chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d").map_err(|_| invalid())?;
                if date.to_string() != text {
                    return Err(invalid());
                }
                dates.push(date);
            }
        }
        if dates.len() == 2 && dates[0] > dates[1] {
            return Err(invalid());
        }
        Ok(value)
    }
}
