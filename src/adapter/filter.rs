use crate::config;
use passage_adapters::filter::FilterAdapter;
use passage_adapters::{FixedFilterAdapter, Protocol, Target};
use sentry::protocol::Uuid;
use std::net::SocketAddr;

#[derive(Debug)]
pub enum DynFilterAdapter {
    Fixed(FixedFilterAdapter),
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
        config: config::FilterAdapter,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        #[allow(unreachable_patterns)]
        match config {
            config::FilterAdapter::Fixed(_config) => {
                let adapter = FixedFilterAdapter::new();
                Ok(DynFilterAdapter::Fixed(adapter))
            }
            _ => Err("unknown filter adapter configured".into()),
        }
    }

    pub async fn from_configs(
        configs: Vec<config::FilterAdapter>,
    ) -> Result<Vec<Self>, Box<dyn std::error::Error>> {
        let mut filters = Vec::with_capacity(configs.len());
        for config in configs {
            filters.push(Self::from_config(config).await?);
        }
        Ok(filters)
    }
}
