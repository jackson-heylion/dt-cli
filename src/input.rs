//! Bounded parameter input and unambiguous JSON, shared by both providers.
use crate::{
    catalog::string,
    output::{Result, invalid},
};
use serde_json::{Map, Value, json};

pub(crate) fn read_parameters(leaf: &clap::ArgMatches) -> Result<Option<String>> {
    if string(leaf, "params").is_some() && string(leaf, "params-file").is_some() {
        return Err(invalid());
    }
    let raw = if let Some(path) = string(leaf, "params-file") {
        if path == "-" {
            use std::io::Read;
            let mut s = String::new();
            std::io::stdin()
                .take(65537)
                .read_to_string(&mut s)
                .map_err(|_| invalid())?;
            Some(s)
        } else {
            use std::io::Read;
            let mut text = String::new();
            std::fs::File::open(path)
                .map_err(|_| invalid())?
                .take(65537)
                .read_to_string(&mut text)
                .map_err(|_| invalid())?;
            Some(text)
        }
    } else {
        string(leaf, "params").map(str::to_owned)
    };
    if raw.as_ref().is_some_and(|v| v.len() > 65536) {
        return Err(invalid());
    }
    Ok(raw)
}
/// JSON parsing that rejects duplicate object keys at any depth.
pub(crate) fn strict_json(raw: &str) -> Result<Value> {
    use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
    struct Seed;
    impl<'de> DeserializeSeed<'de> for Seed {
        type Value = Value;
        fn deserialize<D: serde::Deserializer<'de>>(
            self,
            de: D,
        ) -> std::result::Result<Value, D::Error> {
            de.deserialize_any(StrictVisitor)
        }
    }
    struct StrictVisitor;
    impl<'de> Visitor<'de> for StrictVisitor {
        type Value = Value;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            write!(f, "JSON value")
        }
        fn visit_bool<E: serde::de::Error>(self, v: bool) -> std::result::Result<Value, E> {
            Ok(Value::Bool(v))
        }
        fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Value, E> {
            Ok(json!(v))
        }
        fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Value, E> {
            Ok(json!(v))
        }
        fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Value, E> {
            serde_json::Number::from_f64(v)
                .map(Value::Number)
                .ok_or_else(|| E::custom("invalid number"))
        }
        fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<Value, E> {
            Ok(json!(v))
        }
        fn visit_string<E: serde::de::Error>(self, v: String) -> std::result::Result<Value, E> {
            Ok(json!(v))
        }
        fn visit_none<E: serde::de::Error>(self) -> std::result::Result<Value, E> {
            Ok(Value::Null)
        }
        fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Value, E> {
            Ok(Value::Null)
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Value, A::Error> {
            let mut items = Vec::new();
            while let Some(item) = seq.next_element_seed(Seed)? {
                items.push(item);
            }
            Ok(Value::Array(items))
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Value, A::Error> {
            let mut entries = Map::new();
            while let Some(key) = map.next_key::<String>()? {
                if entries.contains_key(&key) {
                    return Err(serde::de::Error::custom("duplicate JSON key"));
                }
                let value = map.next_value_seed(Seed)?;
                entries.insert(key, value);
            }
            Ok(Value::Object(entries))
        }
    }
    let mut de = serde_json::Deserializer::from_str(raw);
    let value = Seed.deserialize(&mut de).map_err(|_| invalid())?;
    de.end().map_err(|_| invalid())?;
    Ok(value)
}
