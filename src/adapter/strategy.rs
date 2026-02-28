use crate::config;
use passage_adapters::strategy::StrategyAdapter;
use passage_adapters::{AnyStrategyAdapter, PlayerFillStrategyAdapter, Protocol, Target};
#[cfg(feature = "adapters-grpc")]
use passage_adapters_grpc::GrpcStrategyAdapter;
use sentry::protocol::Uuid;
use std::fmt::{Display, Formatter};
use std::net::SocketAddr;

#[derive(Debug)]
pub enum DynStrategyAdapter {
    Any(AnyStrategyAdapter),
    PlayerFill(PlayerFillStrategyAdapter),
    #[cfg(feature = "adapters-grpc")]
    Grpc(GrpcStrategyAdapter),
}

impl Display for DynStrategyAdapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Any(_) => write!(f, "any"),
            Self::PlayerFill(_) => write!(f, "player_fill"),
            #[cfg(feature = "adapters-grpc")]
            Self::Grpc(_) => write!(f, "grpc"),
        }
    }
}

impl StrategyAdapter for DynStrategyAdapter {
    async fn strategize(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        user: (&str, &Uuid),
        targets: Vec<Target>,
    ) -> passage_adapters::Result<Option<Target>> {
        match self {
            DynStrategyAdapter::Any(adapter) => {
                adapter
                    .strategize(client_addr, server_addr, protocol, user, targets)
                    .await
            }
            DynStrategyAdapter::PlayerFill(adapter) => {
                adapter
                    .strategize(client_addr, server_addr, protocol, user, targets)
                    .await
            }
            #[cfg(feature = "adapters-grpc")]
            DynStrategyAdapter::Grpc(adapter) => {
                adapter
                    .strategize(client_addr, server_addr, protocol, user, targets)
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
            config::StrategyAdapter::Any => {
                let adapter = AnyStrategyAdapter::new();
                Ok(DynStrategyAdapter::Any(adapter))
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
