use crate::config;
use passage_adapters::strategy::StrategyAdapter;
use passage_adapters::{FixedStrategyAdapter, PlayerFillStrategyAdapter, Protocol, Target};
#[cfg(feature = "adapters-grpc")]
use passage_adapters_grpc::GrpcStrategyAdapter;
use sentry::protocol::Uuid;
use std::net::SocketAddr;

#[derive(Debug)]
pub enum DynStrategyAdapter {
    Fixed(FixedStrategyAdapter),
    PlayerFill(PlayerFillStrategyAdapter),
    #[cfg(feature = "adapters-grpc")]
    Grpc(GrpcStrategyAdapter),
}

impl StrategyAdapter for DynStrategyAdapter {
    async fn select(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        user: (&str, &Uuid),
        targets: Vec<Target>,
    ) -> passage_adapters::Result<Option<Target>> {
        match self {
            DynStrategyAdapter::Fixed(adapter) => {
                adapter
                    .select(client_addr, server_addr, protocol, user, targets)
                    .await
            }
            DynStrategyAdapter::PlayerFill(adapter) => {
                adapter
                    .select(client_addr, server_addr, protocol, user, targets)
                    .await
            }
            #[cfg(feature = "adapters-grpc")]
            DynStrategyAdapter::Grpc(adapter) => {
                adapter
                    .select(client_addr, server_addr, protocol, user, targets)
                    .await
            }
        }
    }
}

impl DynStrategyAdapter {
    pub async fn from_config(
        config: config::StrategyAdapter,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        #[allow(unreachable_patterns)]
        match config {
            config::StrategyAdapter::Fixed(_config) => {
                let adapter = FixedStrategyAdapter::new();
                Ok(DynStrategyAdapter::Fixed(adapter))
            }
            config::StrategyAdapter::PlayerFill(config) => {
                let adapter = PlayerFillStrategyAdapter::new(config.field, config.max_players);
                Ok(DynStrategyAdapter::PlayerFill(adapter))
            }
            #[cfg(feature = "adapters-grpc")]
            config::StrategyAdapter::Grpc(config) => {
                let adapter = GrpcStrategyAdapter::new(config.address).await?;
                Ok(DynStrategyAdapter::Grpc(adapter))
            }
            _ => Err("unknown strategy adapter configured".into()),
        }
    }
}
