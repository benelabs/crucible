use serde_json::{Map, Value};

pub struct DataTransformer;

impl DataTransformer {
    pub fn new() -> Self {
        Self {}
    }

    /// Apply a simple normalization pipeline to a JSON value.
    /// - If the value is an object, convert all top-level keys to lowercase.
    /// - Optionally extract a nested field by dot-path using `extract_field`.
    pub fn transform(&self, mut input: Value) -> Value {
        if let Value::Object(map) = &mut input {
            let mut normalized = Map::new();
            for (k, v) in std::mem::take(map).into_iter() {
                normalized.insert(k.to_lowercase(), v);
            }
            return Value::Object(normalized);
        }
        input
    }

    /// Extract a nested field from `value` using a dot-separated path like "a.b.c".
    pub fn extract_field<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
        let mut current = value;
        for part in path.split('.') {
            match current {
                Value::Object(map) => {
                    current = map.get(part)?;
                }
                _ => return None,
            }
        }
        Some(current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_keys() {
        let t = DataTransformer::new();
        let input: Value = serde_json::json!({ "Name": "Alice", "AGE": 30 });
        let out = t.transform(input);
        if let Value::Object(map) = out {
            assert!(map.contains_key("name"));
            assert!(map.contains_key("age"));
        } else {
            panic!("expected object")
        }
    }

    #[test]
    fn extract_dot_path() {
        let v: Value = serde_json::json!({ "a": { "b": { "c": 1 } } });
        let got = DataTransformer::extract_field(&v, "a.b.c").unwrap();
        assert_eq!(got, &Value::Number(1.into()));
    }
}
