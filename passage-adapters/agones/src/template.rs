use serde_json::Value;
use std::collections::HashMap;

pub type TemplateValues = HashMap<String, Value>;

/// A template that can be used to replace values in a JSON object. This replacement is (currently)
/// very naive. It only replaces full strings (not a particular format). In the future, this may be
/// replaced with something more sophisticated.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Default)]
pub struct Template {
    #[serde(flatten)]
    inner: Value,
}

impl Template {
    pub fn new(inner: Value) -> Self {
        Self { inner }
    }

    /// Creates a new [`Value`] from this by replacing all template values with the given values.
    pub fn template(&self, values: &TemplateValues) -> Value {
        Self::replace_in(self.inner.clone(), values)
    }

    /// Recursively replaces all template values in the given value with the given values.
    fn replace_in(inner: Value, values: &TemplateValues) -> Value {
        match inner {
            // Try to replace the value from the replacement map.
            Value::String(inner) => values.get(&inner).cloned().unwrap_or(Value::String(inner)),
            // Recursively replace all array and object values.
            Value::Array(inner) => {
                let inner = inner
                    .into_iter()
                    .map(|inner| Self::replace_in(inner, values))
                    .collect();
                Value::Array(inner)
            }
            Value::Object(inner) => {
                let inner = inner
                    .into_iter()
                    .map(|(key, inner)| (key, Self::replace_in(inner, values)))
                    .collect();
                Value::Object(inner)
            }
            // All (non-string) primitives cannot be replaced.
            value => value,
        }
    }
}
