use crate::adapter::{opt_to_regex, opt_vec_to_uuid};
use crate::config;
use passage_adapters::filter::FilterAdapter;
use passage_adapters::filter::meta::{FilterOperation, FilterRule};
use passage_adapters::{
    MetaFilterAdapter, OptionFilterAdapter, PlayerAllowFilterAdapter, PlayerBlockFilterAdapter,
    Protocol, Target,
};
use sentry::protocol::Uuid;
use std::fmt::{Display, Formatter};
use std::net::SocketAddr;

#[derive(Debug)]
pub enum DynFilterAdapter {
    Meta(OptionFilterAdapter<MetaFilterAdapter>),
    PlayerAllow(OptionFilterAdapter<PlayerAllowFilterAdapter>),
    PlayerBlock(OptionFilterAdapter<PlayerBlockFilterAdapter>),
}

impl Display for DynFilterAdapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Meta(_) => write!(f, "meta"),
            Self::PlayerAllow(_) => write!(f, "player_allow"),
            Self::PlayerBlock(_) => write!(f, "player_block"),
        }
    }
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
            DynFilterAdapter::Meta(adapter) => {
                adapter
                    .filter(client_addr, server_addr, protocol, user, targets)
                    .await
            }
            DynFilterAdapter::PlayerAllow(adapter) => {
                adapter
                    .filter(client_addr, server_addr, protocol, user, targets)
                    .await
            }
            DynFilterAdapter::PlayerBlock(adapter) => {
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
            config::FilterAdapter::Meta(config) => {
                let adapter =
                    MetaFilterAdapter::new(config.rules.into_iter().map(Into::into).collect());
                let option_adapter = OptionFilterAdapter::new(hostname, adapter)?;
                Ok(DynFilterAdapter::Meta(option_adapter))
            }
            config::FilterAdapter::PlayerAllow(config) => {
                let adapter = PlayerAllowFilterAdapter::new(
                    config.usernames,
                    opt_to_regex(config.username)?,
                    opt_vec_to_uuid(config.ids)?,
                );
                let option_adapter = OptionFilterAdapter::new(hostname, adapter)?;
                Ok(DynFilterAdapter::PlayerAllow(option_adapter))
            }
            config::FilterAdapter::PlayerBlock(config) => {
                let adapter = PlayerBlockFilterAdapter::new(
                    config.usernames,
                    opt_to_regex(config.username)?,
                    opt_vec_to_uuid(config.ids)?,
                );
                let option_adapter = OptionFilterAdapter::new(hostname, adapter)?;
                Ok(DynFilterAdapter::PlayerBlock(option_adapter))
            }
            _ => Err("unknown filter adapter configured".into()),
        }
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

#[derive(Debug)]
pub struct DynFilterAdapters {
    filters: Vec<DynFilterAdapter>,
}

impl Display for DynFilterAdapters {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "[")?;
        for (i, adapter) in self.filters.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", adapter)?;
        }
        write!(f, "]")
    }
}

impl FilterAdapter for DynFilterAdapters {
    async fn filter(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        user: (&str, &Uuid),
        targets: Vec<Target>,
    ) -> passage_adapters::Result<Vec<Target>> {
        let mut filtered_targets = targets;
        for filter in &self.filters {
            filtered_targets = filter
                .filter(client_addr, server_addr, protocol, user, filtered_targets)
                .await?;
        }
        Ok(filtered_targets)
    }
}

impl DynFilterAdapters {
    pub async fn from_config(
        configs: Vec<config::OptionFilterAdapter>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut filters = Vec::with_capacity(configs.len());
        for config in configs {
            filters.push(DynFilterAdapter::from_config(config).await?);
        }
        Ok(DynFilterAdapters { filters })
    }
}
