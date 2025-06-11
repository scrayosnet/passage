use crate::adapter::proto::strategy_client::StrategyClient;
use crate::adapter::proto::{Address, SelectRequest};
use crate::adapter::status::Protocol;
use crate::adapter::target_selection::Target;
use crate::adapter::target_strategy::TargetSelectorStrategy;
use crate::adapter::Error;
use crate::config::GrpcTargetStrategy as GrpcConfig;
use async_trait::async_trait;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use tonic::transport::Channel;
use uuid::Uuid;

pub struct GrpcTargetSelectorStrategy {
    client: StrategyClient<Channel>,
}

impl GrpcTargetSelectorStrategy {
    pub async fn new(config: GrpcConfig) -> Result<Self, Error> {
        Ok(Self {
            client: StrategyClient::connect(config.address)
                .await
                .map_err(|err| Error::FailedInitialization {
                    adapter_type: "target_strategy",
                    cause: err.into(),
                })?,
        })
    }
}

#[async_trait]
impl TargetSelectorStrategy for GrpcTargetSelectorStrategy {
    async fn select(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        username: &str,
        user_id: &Uuid,
        targets: &[Target],
    ) -> Result<Option<SocketAddr>, Error> {
        let request = tonic::Request::new(SelectRequest {
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
            targets: targets.iter().map(Into::into).collect(),
        });
        let response = self.client.clone().select_target(request).await?;

        // return the result right away
        Ok(response
            .into_inner()
            .address
            .map(TryInto::try_into)
            .transpose()?)
    }
}

impl TryFrom<Address> for SocketAddr {
    type Error = Error;

    fn try_from(value: Address) -> Result<Self, Self::Error> {
        Ok(Self::new(
            IpAddr::from_str(&value.hostname)?,
            u16::try_from(value.port)?,
        ))
    }
}
