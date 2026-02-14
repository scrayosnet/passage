use crate::proto::strategy_client::StrategyClient;
use crate::proto::{Address, SelectRequest};
use passage_adapters::strategy::StrategyAdapter;
use passage_adapters::{Error, Protocol, Target};
use std::fmt::{Debug, Formatter};
use std::net::SocketAddr;
use tonic::transport::Channel;
use tracing::instrument;
use uuid::Uuid;

pub struct GrpcStrategyAdapter {
    client: StrategyClient<Channel>,
}

impl Debug for GrpcStrategyAdapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "GrpcStrategyAdapter")
    }
}

impl GrpcStrategyAdapter {
    pub async fn new<D>(address: D) -> Result<Self, Error>
    where
        D: TryInto<tonic::transport::Endpoint>,
        D::Error: Into<tonic::codegen::StdError>,
    {
        Ok(Self {
            client: StrategyClient::connect(address).await.map_err(|err| {
                Error::FailedInitialization {
                    adapter_type: "target_strategy",
                    cause: err.into(),
                }
            })?,
        })
    }
}

impl StrategyAdapter for GrpcStrategyAdapter {
    #[instrument(skip_all)]
    async fn select(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        user: (&str, &Uuid),
        targets: Vec<Target>,
    ) -> Result<Option<Target>, Error> {
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
            username: user.0.to_string(),
            user_id: user.1.to_string(),
            targets: targets.iter().map(Into::into).collect(),
        });
        let response = self
            .client
            .clone()
            .select_target(request)
            .await
            .map_err(|err| Error::FailedFetch {
                adapter_type: "grpc_target_strategy",
                cause: err.into(),
            })?;

        // return the result right away
        response
            .into_inner()
            .target
            .map(TryInto::try_into)
            .transpose()
    }
}
