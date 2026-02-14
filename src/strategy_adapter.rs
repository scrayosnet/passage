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
        username: &str,
        user_id: &Uuid,
        targets: Vec<Target>,
    ) -> passage_adapters::Result<Option<Target>> {
        match self {
            DynStrategyAdapter::Fixed(adapter) => {
                adapter
                    .select(
                        client_addr,
                        server_addr,
                        protocol,
                        username,
                        user_id,
                        targets,
                    )
                    .await
            }
            DynStrategyAdapter::PlayerFill(adapter) => {
                adapter
                    .select(
                        client_addr,
                        server_addr,
                        protocol,
                        username,
                        user_id,
                        targets,
                    )
                    .await
            }
            #[cfg(feature = "adapters-grpc")]
            DynStrategyAdapter::Grpc(adapter) => {
                adapter
                    .select(
                        client_addr,
                        server_addr,
                        protocol,
                        username,
                        user_id,
                        targets,
                    )
                    .await
            }
        }
    }
}

impl DynStrategyAdapter {
    pub async fn from_config(
        config: &config::TargetStrategy,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        match config.adapter.as_str() {
            "fixed" => {
                let Some(_config) = config.fixed.clone() else {
                    return Err("fixed strategy adapter requires a configuration".into());
                };
                let adapter = FixedStrategyAdapter::new();
                Ok(DynStrategyAdapter::Fixed(adapter))
            }
            "player_fill" => {
                let Some(config) = config.player_fill.clone() else {
                    return Err("fixed strategy adapter requires a configuration".into());
                };
                let adapter = PlayerFillStrategyAdapter::new(config.field, config.max_players);
                Ok(DynStrategyAdapter::PlayerFill(adapter))
            }
            #[cfg(feature = "adapters-grpc")]
            "grpc" => {
                let Some(config) = config.grpc.clone() else {
                    return Err("grpc strategy adapter requires a configuration".into());
                };
                let adapter = GrpcStrategyAdapter::new(config.address).await?;
                Ok(DynStrategyAdapter::Grpc(adapter))
            }
            _ => Err("unknown strategy adapter configured".into()),
        }
    }
}
