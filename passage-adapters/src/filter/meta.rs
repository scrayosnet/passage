use crate::filter::FilterAdapter;
use crate::{Protocol, Target, error::Result};
use std::net::SocketAddr;
use tracing::trace;
use uuid::Uuid;

/// Filter operation to apply to a target field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterOperation {
    /// Field must equal the specified value.
    Equals(String),
    /// Field must not equal the specified value.
    NotEquals(String),
    /// Field must exist (have any value).
    Exists,
    /// Field must not exist.
    NotExists,
    /// Field must be one of the specified values.
    In(Vec<String>),
    /// Field must not be any of the specified values.
    NotIn(Vec<String>),
}

impl FilterOperation {
    /// Check if a field value matches this filter operation.
    fn matches(&self, field_value: Option<&str>) -> bool {
        match self {
            FilterOperation::Equals(value) => field_value == Some(value.as_str()),
            FilterOperation::NotEquals(value) => field_value != Some(value.as_str()),
            FilterOperation::Exists => field_value.is_some(),
            FilterOperation::NotExists => field_value.is_none(),
            FilterOperation::In(values) => {
                field_value.is_some_and(|v| values.iter().any(|val| val == v))
            }
            FilterOperation::NotIn(values) => {
                field_value.is_none()
                    || field_value.is_some_and(|v| !values.iter().any(|val| val == v))
            }
        }
    }
}

/// A single filter rule.
#[derive(Debug, Clone)]
pub struct FilterRule {
    /// The metadata key to filter on.
    pub key: String,
    /// The operation to perform.
    pub operation: FilterOperation,
}

impl FilterRule {
    /// Check if a target matches this filter rule.
    fn matches(&self, target: &Target) -> bool {
        let field_value = target.meta.get(&self.key).map(|s| s.as_str());
        self.operation.matches(field_value)
    }
}

#[derive(Debug, Default)]
pub struct MetaFilterAdapter {
    /// List of filter rules. All rules must match (AND logic).
    rules: Vec<FilterRule>,
}

impl MetaFilterAdapter {
    pub fn new(rules: Vec<FilterRule>) -> Self {
        Self { rules }
    }

    /// Add a filter rule to this adapter.
    pub fn add_rule(mut self, key: String, operation: FilterOperation) -> Self {
        self.rules.push(FilterRule { key, operation });
        self
    }

    /// Check if a target matches all filter rules.
    pub fn matches_filters(&self, target: &Target) -> bool {
        // Empty rules means accept all targets
        if self.rules.is_empty() {
            return true;
        }

        // All rules must match (AND logic)
        self.rules.iter().all(|rule| rule.matches(target))
    }
}

impl FilterAdapter for MetaFilterAdapter {
    #[tracing::instrument(skip_all)]
    async fn filter(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
        _user: (&str, &Uuid),
        targets: Vec<Target>,
    ) -> Result<Vec<Target>> {
        trace!(
            len = targets.len(),
            rules = self.rules.len(),
            "filtering targets"
        );

        // apply filters
        let filtered: Vec<Target> = targets
            .into_iter()
            .filter(|target| self.matches_filters(target))
            .collect();

        trace!(filtered_len = filtered.len(), "filtering complete");

        Ok(filtered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_target(id: &str, meta: Vec<(&str, &str)>) -> Target {
        Target {
            identifier: id.to_string(),
            address: "127.0.0.1:8080".parse().unwrap(),
            meta: meta
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn test_equals_filter() {
        let filter = MetaFilterAdapter::default().add_rule(
            "region".to_string(),
            FilterOperation::Equals("us-west".to_string()),
        );

        let target1 = create_target("t1", vec![("region", "us-west")]);
        let target2 = create_target("t2", vec![("region", "us-east")]);

        assert!(filter.matches_filters(&target1));
        assert!(!filter.matches_filters(&target2));
    }

    #[test]
    fn test_not_equals_filter() {
        let filter = MetaFilterAdapter::default().add_rule(
            "region".to_string(),
            FilterOperation::NotEquals("us-west".to_string()),
        );

        let target1 = create_target("t1", vec![("region", "us-west")]);
        let target2 = create_target("t2", vec![("region", "us-east")]);

        assert!(!filter.matches_filters(&target1));
        assert!(filter.matches_filters(&target2));
    }

    #[test]
    fn test_exists_filter() {
        let filter =
            MetaFilterAdapter::default().add_rule("region".to_string(), FilterOperation::Exists);

        let target1 = create_target("t1", vec![("region", "us-west")]);
        let target2 = create_target("t2", vec![]);

        assert!(filter.matches_filters(&target1));
        assert!(!filter.matches_filters(&target2));
    }

    #[test]
    fn test_not_exists_filter() {
        let filter =
            MetaFilterAdapter::default().add_rule("region".to_string(), FilterOperation::NotExists);

        let target1 = create_target("t1", vec![("region", "us-west")]);
        let target2 = create_target("t2", vec![]);

        assert!(!filter.matches_filters(&target1));
        assert!(filter.matches_filters(&target2));
    }

    #[test]
    fn test_in_filter() {
        let filter = MetaFilterAdapter::default().add_rule(
            "region".to_string(),
            FilterOperation::In(vec!["us-west".to_string(), "us-east".to_string()]),
        );

        let target1 = create_target("t1", vec![("region", "us-west")]);
        let target2 = create_target("t2", vec![("region", "eu-west")]);

        assert!(filter.matches_filters(&target1));
        assert!(!filter.matches_filters(&target2));
    }

    #[test]
    fn test_multiple_rules() {
        let filter = MetaFilterAdapter::default()
            .add_rule(
                "region".to_string(),
                FilterOperation::Equals("us-west".to_string()),
            )
            .add_rule(
                "environment".to_string(),
                FilterOperation::Equals("production".to_string()),
            );

        let target1 = create_target(
            "t1",
            vec![("region", "us-west"), ("environment", "production")],
        );
        let target2 = create_target(
            "t2",
            vec![("region", "us-west"), ("environment", "staging")],
        );
        let target3 = create_target(
            "t3",
            vec![("region", "us-east"), ("environment", "production")],
        );

        assert!(filter.matches_filters(&target1));
        assert!(!filter.matches_filters(&target2));
        assert!(!filter.matches_filters(&target3));
    }

    #[test]
    fn test_empty_rules_accepts_all() {
        let filter = MetaFilterAdapter::default();

        let target = create_target("t1", vec![("region", "us-west")]);

        assert!(filter.matches_filters(&target));
    }
}
