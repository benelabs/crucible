use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{json, Value};

/// Generates a JSON schema for the given type T.
///
/// The type T must implement `JsonSchema` and `Serialize`.
/// This function uses the `schemars` crate to generate the schema.
///
/// # Example
///
/// ```
/// use backend::utils::json_schema::generate_json_schema;
/// use serde::{Deserialize, Serialize};
/// use schemars::JsonSchema;
///
/// #[derive(Serialize, Deserialize, JsonSchema)]
/// struct Person {
///     name: String,
///     age: u32,
/// }
///
/// let schema = generate_json_schema::<Person>();
/// println!("{}", serde_json::to_string_pretty(&schema).unwrap());
/// ```
pub fn generate_json_schema<T>() -> Value
where
    T: JsonSchema + Serialize,
{
    let schema = schemars::schema_for!(T);
    serde_json::to_value(schema).unwrap_or_else(|_| json!({"error": "Failed to serialize schema"}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize, JsonSchema)]
    struct TestStruct {
        name: String,
        age: u32,
        active: bool,
    }

    #[test]
    fn test_generate_json_schema() {
        let schema = generate_json_schema::<TestStruct>();
        assert!(schema.is_object());
        assert_eq!(
            schema["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"].is_object());
        assert!(schema["properties"]["name"].is_object());
        assert!(schema["properties"]["age"].is_object());
        assert!(schema["properties"]["active"].is_object());
    }

    #[derive(Serialize, Deserialize, JsonSchema)]
    struct NestedStruct {
        person: TestStruct,
        count: i64,
    }

    #[test]
    fn test_generate_nested_schema() {
        let schema = generate_json_schema::<NestedStruct>();
        assert!(schema.is_object());
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["person"].is_object());
        assert!(schema["properties"]["count"].is_object());
    }
}
