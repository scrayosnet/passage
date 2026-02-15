use crate::config;
use passage_adapters::filter::FilterAdapter;
use passage_adapters::filter::fixed::{FilterOperation, FilterRule};
use passage_adapters::{FixedFilterAdapter, OptionFilterAdapter, Protocol, Target};
use sentry::protocol::Uuid;
use std::net::SocketAddr;

#[derive(Debug)]
pub enum DynFilterAdapter {
    Fixed(OptionFilterAdapter<FixedFilterAdapter>),
}

impl FilterAdapter for DynFilterAdapter {
    async fn filter(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        user: (&str, &Uuid),
        targets: Vec<Target>,
    ) -> passage_adapters::Result<Vec<Target>> {
        match self {
            DynFilterAdapter::Fixed(adapter) => {
                adapter
                    .filter(client_addr, server_addr, protocol, user, targets)
                    .await
            }
        }
    }
}

impl DynFilterAdapter {
    pub async fn from_config(
        config: config::OptionFilterAdapter,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let hostname = config.hostname;
        #[allow(unreachable_patterns)]
        match config.filter {
            config::FilterAdapter::Fixed(config) => {
                let adapter =
                    FixedFilterAdapter::new(config.rules.into_iter().map(Into::into).collect());
                let option_adapter = OptionFilterAdapter::new(hostname, adapter)?;
                Ok(DynFilterAdapter::Fixed(option_adapter))
            }
            _ => Err("unknown filter adapter configured".into()),
        }
    }

    pub async fn from_configs(
        configs: Vec<config::OptionFilterAdapter>,
    ) -> Result<Vec<Self>, Box<dyn std::error::Error>> {
        let mut filters = Vec::with_capacity(configs.len());
        for config in configs {
            filters.push(Self::from_config(config).await?);
        }
        Ok(filters)
    }
}

impl From<config::FilterRule> for FilterRule {
    fn from(value: config::FilterRule) -> Self {
        Self {
            key: value.key,
            operation: value.operation.into(),
        }
    }
}

impl From<config::FilterOperation> for FilterOperation {
    fn from(value: config::FilterOperation) -> Self {
        match value {
            config::FilterOperation::Equals(value) => FilterOperation::Equals(value),
            config::FilterOperation::NotEquals(value) => FilterOperation::NotEquals(value),
            config::FilterOperation::Exists => FilterOperation::Exists,
            config::FilterOperation::NotExists => FilterOperation::NotExists,
            config::FilterOperation::In(values) => FilterOperation::In(values),
            config::FilterOperation::NotIn(values) => FilterOperation::NotIn(values),
        }
    }
}
