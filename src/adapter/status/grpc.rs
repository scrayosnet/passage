use crate::adapter::proto::status_client::StatusClient;
use crate::adapter::proto::{Address, Players, ProtocolVersion, StatusData, StatusRequest};
use crate::adapter::status::{
    Protocol, ServerPlayer, ServerPlayers, ServerStatus, ServerVersion, StatusSupplier,
};
use crate::adapter::Error;
use crate::config::GrpcStatus as GrpcConfig;
use async_trait::async_trait;
use serde_json::value::RawValue;
use std::net::SocketAddr;
use tonic::transport::Channel;

pub struct GrpcStatusSupplier {
    client: StatusClient<Channel>,
}

impl GrpcStatusSupplier {
    pub async fn new(config: GrpcConfig) -> Result<Self, Error> {
        Ok(Self {
            client: StatusClient::connect(config.address).await.map_err(|err| {
                Error::FailedInitialization {
                    adapter_type: "status",
                    cause: err.into(),
                }
            })?,
        })
    }
}

#[async_trait]
impl StatusSupplier for GrpcStatusSupplier {
    async fn get_status(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
    ) -> Result<Option<ServerStatus>, Error> {
        let request = tonic::Request::new(StatusRequest {
            client_address: Some(Address {
                hostname: client_addr.ip().to_string(),
                port: u32::from(client_addr.port()),
            }),
            server_address: Some(Address {
                hostname: server_addr.0.to_string(),
                port: u32::from(server_addr.1),
            }),
            protocol: protocol as u64,
        });
        let response = self.client.clone().get_status(request).await?;

        Ok(response
            .into_inner()
            .status
            .map(TryInto::try_into)
            .transpose()?)
    }
}

impl TryFrom<StatusData> for ServerStatus {
    type Error = Error;

    fn try_from(value: StatusData) -> Result<Self, Self::Error> {
        let description = value.description.map(RawValue::from_string).transpose()?;
        let favicon = value.favicon.map(String::from_utf8).transpose()?;

        Ok(Self {
            version: value.version.map(Into::into).ok_or(Error::MissingData {
                field: "status.version",
            })?,
            players: value.players.map(Into::into),
            description,
            favicon,
            enforces_secure_chat: value.enforces_secure_chat,
        })
    }
}

impl From<ProtocolVersion> for ServerVersion {
    fn from(value: ProtocolVersion) -> Self {
        Self {
            name: value.name,
            protocol: value.protocol,
        }
    }
}

impl From<Players> for ServerPlayers {
    fn from(value: Players) -> Self {
        let samples: Option<Vec<ServerPlayer>> = if value.samples.is_empty() {
            None
        } else {
            Some(
                value
                    .samples
                    .iter()
                    .map(|raw| ServerPlayer {
                        name: raw.name.clone(),
                        id: raw.id.clone(),
                    })
                    .collect(),
            )
        };

        Self {
            online: value.online,
            max: value.max,
            sample: samples,
        }
    }
}
