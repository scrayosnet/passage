use crate::config;
use passage_adapters::status::StatusAdapter;
use passage_adapters::{FixedStatusAdapter, Protocol, ServerStatus, ServerVersion};
#[cfg(feature = "adapters-grpc")]
use passage_adapters_grpc::GrpcStatusAdapter;
#[cfg(feature = "adapters-http")]
use passage_adapters_http::HttpStatusAdapter;
use serde_json::value::RawValue;
use std::net::SocketAddr;

#[derive(Debug)]
pub enum DynStatusAdapter {
    Fixed(FixedStatusAdapter),
    #[cfg(feature = "adapters-grpc")]
    Grpc(GrpcStatusAdapter),
    #[cfg(feature = "adapters-http")]
    Http(HttpStatusAdapter),
}

impl StatusAdapter for DynStatusAdapter {
    async fn status(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
    ) -> passage_adapters::Result<Option<ServerStatus>> {
        match self {
            DynStatusAdapter::Fixed(adapter) => {
                adapter.status(client_addr, server_addr, protocol).await
            }
            #[cfg(feature = "adapters-grpc")]
            DynStatusAdapter::Grpc(adapter) => {
                adapter.status(client_addr, server_addr, protocol).await
            }
            #[cfg(feature = "adapters-http")]
            DynStatusAdapter::Http(adapter) => {
                adapter.status(client_addr, server_addr, protocol).await
            }
        }
    }
}

impl DynStatusAdapter {
    pub async fn from_config(
        config: config::StatusAdapter,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        #[allow(unreachable_patterns)]
        match config {
            config::StatusAdapter::Fixed(config) => {
                let description = config
                    .description
                    .and_then(|str| RawValue::from_string(str).ok());
                let adapter = FixedStatusAdapter::new(
                    Some(ServerStatus {
                        version: ServerVersion {
                            name: config.name,
                            protocol: 0,
                        },
                        players: None,
                        description,
                        favicon: config.favicon,
                        enforces_secure_chat: config.enforces_secure_chat,
                    }),
                    config.preferred_version,
                    config.min_version,
                    config.max_version,
                );
                Ok(DynStatusAdapter::Fixed(adapter))
            }
            #[cfg(feature = "adapters-grpc")]
            config::StatusAdapter::Grpc(config) => {
                let adapter = GrpcStatusAdapter::new(config.address).await?;
                Ok(DynStatusAdapter::Grpc(adapter))
            }
            #[cfg(feature = "adapters-http")]
            config::StatusAdapter::Http(config) => {
                let adapter = HttpStatusAdapter::new(config.address, config.cache_duration)?;
                Ok(DynStatusAdapter::Http(adapter))
            }
            _ => Err("unknown status adapter configured".into()),
        }
    }
}
