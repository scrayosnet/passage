use crate::proto::status_client::StatusClient;
use crate::proto::{Address, StatusRequest};
use passage_adapters::{Error, Protocol, Result, ServerStatus, metrics, status::StatusAdapter};
use std::fmt::{Debug, Formatter};
use std::net::SocketAddr;
use tokio::time::Instant;
use tonic::transport::Channel;
use tracing::instrument;

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "grpc_status_adapter";

pub struct GrpcStatusAdapter {
    client: StatusClient<Channel>,
}

impl Debug for GrpcStatusAdapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "GrpcStatusAdapter")
    }
}

impl GrpcStatusAdapter {
    pub async fn new<D>(address: D) -> Result<Self, Error>
    where
        D: TryInto<tonic::transport::Endpoint>,
        D::Error: Into<tonic::codegen::StdError>,
    {
        Ok(Self {
            client: StatusClient::connect(address).await.map_err(|err| {
                Error::FailedInitialization {
                    adapter_type: "grpc_status",
                    cause: err.into(),
                }
            })?,
        })
    }

    #[instrument(skip_all)]
    async fn status(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
    ) -> Result<Option<ServerStatus>> {
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

        self.client
            .clone()
            .get_status(request)
            .await
            .map_err(|err| Error::FailedFetch {
                adapter_type: "grpc_status",
                cause: err.into(),
            })?
            .into_inner()
            .status
            .map(TryInto::try_into)
            .transpose()
    }
}

impl StatusAdapter for GrpcStatusAdapter {
    #[instrument(skip_all)]
    async fn status(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
    ) -> Result<Option<ServerStatus>> {
        let start = Instant::now();
        let status = self.status(client_addr, server_addr, protocol).await;
        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        status
    }
}
