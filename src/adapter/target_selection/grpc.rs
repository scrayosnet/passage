use crate::adapter::Error;
use crate::adapter::proto::discovery_client::DiscoveryClient;
use crate::adapter::proto::{Address, TargetRequest};
use crate::adapter::status::Protocol;
use crate::adapter::target_selection::{Target, TargetSelector, strategize};
use crate::adapter::target_strategy::TargetSelectorStrategy;
use crate::config::GrpcTargetDiscovery as GrpcConfig;
use async_trait::async_trait;
use std::net::SocketAddr;
use std::sync::Arc;
use tonic::transport::Channel;
use tracing::instrument;
use uuid::Uuid;

pub struct GrpcTargetSelector {
    strategy: Arc<dyn TargetSelectorStrategy>,
    client: DiscoveryClient<Channel>,
}

impl GrpcTargetSelector {
    pub async fn new(
        strategy: Arc<dyn TargetSelectorStrategy>,
        config: GrpcConfig,
    ) -> Result<Self, Error> {
        Ok(Self {
            strategy,
            client: DiscoveryClient::connect(config.address)
                .await
                .map_err(|err| Error::FailedInitialization {
                    adapter_type: "grpc_target_selection",
                    cause: err.into(),
                })?,
        })
    }
}

#[async_trait]
impl TargetSelector for GrpcTargetSelector {
    #[instrument(skip_all)]
    async fn select(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        username: &str,
        user_id: &Uuid,
    ) -> Result<Option<Target>, Error> {
        let request = tonic::Request::new(TargetRequest {
            client_address: Some(Address {
                hostname: client_addr.ip().to_string(),
                port: u32::from(client_addr.port()),
            }),
            server_address: Some(Address {
                hostname: server_addr.0.to_string(),
                port: u32::from(server_addr.1),
            }),
            protocol: protocol as u64,
            username: username.to_string(),
            user_id: user_id.to_string(),
        });
        let response = self
            .client
            .clone()
            .get_targets(request)
            .await
            .map_err(|err| Error::FailedFetch {
                adapter_type: "grpc_target_selection",
                cause: err.into(),
            })?;

        let targets: Vec<Target> = response
            .into_inner()
            .targets
            .iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<Target>, _>>()?;

        strategize(
            Arc::clone(&self.strategy),
            client_addr,
            server_addr,
            protocol,
            username,
            user_id,
            &targets,
        )
        .await
    }
}
