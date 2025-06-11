use crate::adapter::proto::resourcepack_client::ResourcepackClient;
use crate::adapter::proto::{Address, Pack, PacksRequest};
use crate::adapter::resourcepack::{Resourcepack, ResourcepackSupplier};
use crate::adapter::status::Protocol;
use crate::adapter::Error;
use crate::config::GrpcResourcepack as GrpcConfig;
use async_trait::async_trait;
use std::net::SocketAddr;
use std::str::FromStr;
use tonic::transport::Channel;
use uuid::Uuid;

pub struct GrpcResourcepackSupplier {
    client: ResourcepackClient<Channel>,
}

impl GrpcResourcepackSupplier {
    pub async fn new(config: GrpcConfig) -> Result<Self, Error> {
        Ok(Self {
            client: ResourcepackClient::connect(config.address)
                .await
                .map_err(|err| Error::FailedInitialization {
                    adapter_type: "resourcepack",
                    cause: err.into(),
                })?,
        })
    }
}

#[async_trait]
impl ResourcepackSupplier for GrpcResourcepackSupplier {
    async fn get_resourcepacks(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        username: &str,
        user_id: &Uuid,
        _user_locale: &str,
    ) -> Result<Vec<Resourcepack>, Error> {
        let request = tonic::Request::new(PacksRequest {
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
        let response = self.client.clone().get_packs(request).await?;

        Ok(response
            .into_inner()
            .packs
            .into_iter()
            .map(Resourcepack::try_from)
            .collect::<Result<Vec<Resourcepack>, _>>()?)
    }
}

impl TryFrom<Pack> for Resourcepack {
    type Error = Error;

    fn try_from(value: Pack) -> Result<Self, Self::Error> {
        Ok(Resourcepack {
            uuid: Uuid::from_str(&value.uuid)?,
            url: value.url,
            hash: value.hash,
            forced: value.forced,
            prompt_message: value.prompt_message,
        })
    }
}
