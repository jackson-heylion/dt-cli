//! 传输解码边界：拒绝重复 JSON 键，不承载业务模型。
use crate::output::{Result, invalid};
use serde_json::{Map, Value, json};
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
