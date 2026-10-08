//! IAM's restricted JCS profile: UTF-16 key order, exact safe integers, no references.
use crate::output::{Result, invalid};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub(super) fn digest(value: &Value) -> Result<String> {
    if !value.is_object() || value.get("contractDigest").is_some() {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    encode(value, &mut bytes)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
fn encode(value: &Value, bytes: &mut Vec<u8>) -> Result<()> {
    match value {
        Value::Object(fields) => {
            if fields.contains_key("$ref") || fields.contains_key("$dynamicRef") {
                return Err(invalid());
            }
            let mut fields: Vec<_> = fields.iter().collect();
            fields.sort_by(|(a, _), (b, _)| a.encode_utf16().cmp(b.encode_utf16()));
            bytes.push(b'{');
            for (index, (key, value)) in fields.iter().enumerate() {
                if index != 0 {
                    bytes.push(b',');
                }
                write_string(key, bytes);
                bytes.push(b':');
                encode(value, bytes)?;
            }
            bytes.push(b'}');
        }
        Value::Array(items) => {
            bytes.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index != 0 {
                    bytes.push(b',');
                }
                encode(item, bytes)?;
            }
            bytes.push(b']');
        }
        Value::Number(number) => {
            let integer = number
                .as_i64()
                .filter(|n| n.unsigned_abs() <= 9_007_199_254_740_991)
                .ok_or_else(invalid)?;
            bytes.extend_from_slice(integer.to_string().as_bytes());
        }
        Value::String(value) => write_string(value, bytes),
        _ => serde_json::to_writer(bytes, value).map_err(|_| invalid())?,
    }
    Ok(())
}
fn write_string(value: &str, bytes: &mut Vec<u8>) {
    bytes.push(b'"');
    for c in value.chars() {
        match c {
            '"' => bytes.extend_from_slice(br#"\""#),
            '\\' => bytes.extend_from_slice(br"\\"),
            '\u{8}' => bytes.extend_from_slice(br"\b"),
            '\u{c}' => bytes.extend_from_slice(br"\f"),
            '\n' => bytes.extend_from_slice(br"\n"),
            '\r' => bytes.extend_from_slice(br"\r"),
            '\t' => bytes.extend_from_slice(br"\t"),
            c if c < ' ' => bytes.extend_from_slice(format!("\\u{:04X}", c as u32).as_bytes()),
            c => {
                let mut encoded = [0u8; 4];
                bytes.extend_from_slice(c.encode_utf8(&mut encoded).as_bytes());
            }
        }
    }
    bytes.push(b'"');
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn shared_java_and_rust_argument_digest_vectors_match() {
        let cases: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/task-argument-digests.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            assert_eq!(
                digest(&case["arguments"]).unwrap(),
                case["digest"].as_str().unwrap()
            );
        }
    }
    #[test]
    fn matches_iam_utf16_order_and_exact_integer_profile() {
        let value = json!({"\u{e000}":"中\n\"", "\u{10000}":-4, "a":[true,null]});
        let mut encoded = Vec::new();
        encode(&value, &mut encoded).unwrap();
        assert_eq!(
            String::from_utf8(encoded).unwrap(),
            "{\"a\":[true,null],\"𐀀\":-4,\"\":\"中\\n\\\"\"}"
        );
        assert!(digest(&json!({"n": 1.1})).is_err());
        assert!(digest(&json!({"n": 9007199254740992u64})).is_err());
        assert!(digest(&json!({"nested":{"$ref":"x"}})).is_err());
    }
}
