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
        config: &config::TargetStrategy,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        match config.adapter.as_str() {
            "fixed" => {
                let Some(config) = config.fixed.clone() else {
                    return Err("fixed strategy adapter requires a configuration".into());
                };
                let adapter = FixedFilterAdapter::new();
                Ok(DynFilterAdapter::Fixed(adapter))
            }
            _ => Err("unknown filter adapter configured".into()),
        }
    }
}
