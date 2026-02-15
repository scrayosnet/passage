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
pub struct FixedFilterAdapter {
    /// The hostname to filter on. If set, only targets with this hostname will be filtered. If unset, all targets will be filtered.
    hostname: Option<String>,

    /// List of filter rules. All rules must match (AND logic).
    rules: Vec<FilterRule>,
}

impl FixedFilterAdapter {
    pub fn new(hostname: Option<String>, rules: Vec<FilterRule>) -> Self {
        Self { hostname, rules }
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

impl FilterAdapter for FixedFilterAdapter {
    #[tracing::instrument(skip_all)]
    async fn filter(
        &self,
        _client_addr: &SocketAddr,
        (server_hostname, _): (&str, u16),
        _protocol: Protocol,
        _user: (&str, &Uuid),
        targets: Vec<Target>,
    ) -> Result<Vec<Target>> {
        trace!(
            len = targets.len(),
            rules = self.rules.len(),
            "filtering targets"
        );

        // skip the filter if the hostname doesn't match
        if let Some(hostname) = &self.hostname
            && hostname != server_hostname
        {
            trace!(
                len = targets.len(),
                rules = self.rules.len(),
                "skipping for hostname"
            );
            return Ok(targets);
        }

        // apply filters
        let filtered: Vec<Target> = targets
            .into_iter()
            .filter(|target| self.matches_filters(target))
            .collect();

        trace!(filtered_len = filtered.len(), "filtering complete");

        Ok(filtered)
    }
}
