use crate::proto::strategy_client::StrategyClient;
use crate::proto::{Address, SelectRequest};
use passage_adapters::strategy::StrategyAdapter;
use passage_adapters::{Error, Protocol, Reason, Target, metrics};
use std::fmt::{Debug, Formatter};
use std::net::SocketAddr;
use tokio::time::Instant;
use tonic::transport::Channel;
use tracing::instrument;
use uuid::Uuid;

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "grpc_strategy_adapter";

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
                    adapter_type: ADAPTER_TYPE,
                    cause: err.into(),
                }
            })?,
        })
    }

    #[instrument(skip_all)]
    async fn strategize(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        user: (&str, &Uuid),
        targets: Vec<Target>,
    ) -> Result<Reason<Target>, Error> {
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
                adapter_type: ADAPTER_TYPE,
                cause: err.into(),
            })?;

        // return the result right away
        let Some(target) = response.into_inner().target else {
            return Ok(Reason::None(None));
        };
        Ok(Reason::Some(target.try_into()?))
    }
}

impl StrategyAdapter for GrpcStrategyAdapter {
    #[instrument(skip_all)]
    async fn strategize(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        user: (&str, &Uuid),
        targets: Vec<Target>,
    ) -> Result<Reason<Target>, Error> {
        let start = Instant::now();
        let target = self
            .strategize(client_addr, server_addr, protocol, user, targets)
            .await;
        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        target
    }
}
