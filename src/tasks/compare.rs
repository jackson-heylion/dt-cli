//! Exact value comparison over selected shops; missing and ambiguous rows stay unknown.
use crate::output::{Failure, Result};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
const FIELDS: [&str; 6] = [
    "orderUnit",
    "minOrderNum",
    "maxOrderNum",
    "orderNumMultiplier",
    "enable",
    "orderable",
];

pub(super) fn compare(params: &Value, result: &Value) -> Result<Value> {
    let shops: Vec<&str> = params["shopCodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let selected: BTreeSet<_> = shops.iter().copied().collect();
    let mut rows: BTreeMap<(&str, &str), Vec<&Value>> = BTreeMap::new();
    let mut codes = BTreeSet::new();
    if let Some(code) = params["itemCode"].as_str() {
        codes.insert(code);
    }
    for row in result["items"]
        .as_array()
        .ok_or_else(crate::http::protocol)?
    {
        let shop = row["shopCode"].as_str().ok_or_else(crate::http::protocol)?;
        let code = row["itemCode"].as_str().ok_or_else(crate::http::protocol)?;
        if selected.contains(shop) {
            if params["itemCode"]
                .as_str()
                .is_some_and(|requested| requested != code)
            {
                return Err(Failure::new(
                    "UPSTREAM_CONTRACT_MISMATCH",
                    5,
                    "结果包含不属于本次品项查询的记录。",
                ));
            }
            codes.insert(code);
            rows.entry((shop, code)).or_default().push(row);
        }
    }
    let items:Vec<_>=codes.iter().map(|code| {
        let sources:Vec<_>=shops.iter().map(|shop| {
            let matched=rows.get(&(*shop,*code));
            match matched {
                Some(list) if list.len()==1 => {
                    let row=list[0];
                    let mut values=json!({});
                    for field in FIELDS {values[field]=row[field].clone();}
                    json!({"shopCode":shop,"itemCode":code,"state":"present","itemName":row["itemName"],"values":values})
                }
                Some(list)=>json!({"shopCode":shop,"itemCode":code,"state":"unknown","reason":"duplicate-key","matchingRows":list.len(),"values":Value::Null,"candidates":list.iter().map(|row|{let mut values=json!({});for field in FIELDS{values[field]=row[field].clone();}json!({"itemName":row["itemName"],"values":values})}).collect::<Vec<_>>()}),
                None=>json!({"shopCode":shop,"itemCode":code,"state":"unknown","reason":"no-row","values":Value::Null}),
            }
        }).collect();
        let units:Option<Vec<_>>=sources.iter().map(|r|r["values"]["orderUnit"].as_str().filter(|u|!u.is_empty())).collect();
        let same_unit=units.as_ref().is_some_and(|u|u.iter().all(|v|v==&u[0]));
        let fields:Vec<_>=FIELDS.iter().map(|field| {
            let numeric=matches!(*field,"minOrderNum"|"maxOrderNum"|"orderNumMultiplier");
            let normalized:Option<Vec<_>>=sources.iter().map(|r| {
                if r["state"]!="present" || r["values"][*field].is_null() {return None;}
                if numeric {r["values"][*field].as_str().and_then(decimal)}
                else {Some(r["values"][*field].to_string())}
            }).collect();
            let (state,reason)=match normalized {
                None=>("unknown",Some("missing-or-ambiguous-value")),
                Some(_) if numeric && !same_unit=>("unknown",Some("unit-mismatch-or-unknown")),
                Some(values) if values.iter().all(|v|v==&values[0])=>("same",None),
                Some(_)=>("different",None),
            };
            json!({"field":field,"state":state,"reason":reason})
        }).collect();
        json!({"itemCode":code,"sources":sources,"fields":fields})
    }).collect();
    let missing: Vec<_> = shops
        .iter()
        .filter(|shop| !rows.keys().any(|(s, _)| s == *shop))
        .collect();
    Ok(
        json!({"shopCodes":shops,"items":items,"shopsWithoutRows":missing,
        "comparison":"source-values-only","snapshot":false,"jobId":result["jobId"],"asOf":result["asOf"]}),
    )
}
fn decimal(value: &str) -> Option<String> {
    let (negative, rest) = match value.as_bytes().first()? {
        b'-' => (true, &value[1..]),
        b'+' => (false, &value[1..]),
        _ => (false, value),
    };
    let (integer, fraction) = rest.split_once('.').unwrap_or((rest, ""));
    if integer.is_empty()
        || !integer.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let integer = integer.trim_start_matches('0');
    let integer = if integer.is_empty() { "0" } else { integer };
    let fraction = fraction.trim_end_matches('0');
    let sign = if negative && (integer != "0" || !fraction.is_empty()) {
        "-"
    } else {
        ""
    };
    Some(if fraction.is_empty() {
        format!("{sign}{integer}")
    } else {
        format!("{sign}{integer}.{fraction}")
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    fn row(shop: &str, code: &str, min: Value, unit: Value) -> Value {
        json!({"shopCode":shop,"itemCode":code,"itemName":"同名","orderUnit":unit,"minOrderNum":min,"maxOrderNum":"9007199254740993","orderNumMultiplier":"1.0","enable":true,"orderable":true})
    }
    #[test]
    fn exact_decimals_null_units_and_same_name_items_are_preserved() {
        let p = json!({"itemName":"同名","shopCodes":["a","b","c"]});
        let result = json!({"items":[row("a","1",json!("1.000000000000000001"),json!("件")),row("b","1",json!("1.000000000000000002"),json!("件")),row("c","1",Value::Null,json!("件")),row("a","2",json!("0"),json!("kg")),row("b","2",json!("0.0"),json!("件"))]});
        let v = compare(&p, &result).unwrap();
        assert_eq!(v["items"].as_array().unwrap().len(), 2);
        assert_eq!(v["items"][0]["fields"][1]["state"], "unknown");
        assert_eq!(
            v["items"][0]["sources"][2]["values"]["minOrderNum"],
            Value::Null
        );
        assert_eq!(v["items"][1]["sources"][2]["reason"], "no-row");
        let p = json!({"itemCode":"1","shopCodes":["a","b"]});
        let v = compare(&p, &result).unwrap_err();
        assert_eq!(v.code, "UPSTREAM_CONTRACT_MISMATCH");
        let v=compare(&p,&json!({"items":[row("a","1",json!("1.000000000000000001"),json!("件")),row("b","1",json!("1.000000000000000002"),json!("件"))]})).unwrap();
        assert_eq!(v["items"][0]["fields"][1]["state"], "different");
        assert_eq!(decimal("-000.000"), Some("0".into()));
        assert_eq!(decimal("+001.20"), Some("1.2".into()));
        assert_eq!(decimal("1e2"), None);
    }
    #[test]
    fn duplicate_key_and_mixed_units_cannot_produce_quantity_claims() {
        let p = json!({"itemCode":"1","shopCodes":["a","b"]});
        let a = row("a", "1", json!("0"), json!("kg"));
        let b = row("b", "1", json!("0.00"), json!("件"));
        let v = compare(&p, &json!({"items":[a.clone(),b.clone()]})).unwrap();
        assert_eq!(v["items"][0]["fields"][0]["state"], "different");
        assert_eq!(
            v["items"][0]["fields"][1]["reason"],
            "unit-mismatch-or-unknown"
        );
        let v = compare(&p, &json!({"items":[a.clone(),a,b]})).unwrap();
        assert_eq!(v["items"][0]["sources"][0]["reason"], "duplicate-key");
        assert_eq!(v["items"][0]["fields"][4]["state"], "unknown");
    }
}
