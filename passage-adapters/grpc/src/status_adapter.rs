use crate::proto::status_client::StatusClient;
use crate::proto::{Address, StatusRequest};
use passage_adapters::{Client, Error, Result, ServerStatus, metrics, status::StatusAdapter};
use std::fmt::{Debug, Formatter};
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
    async fn status(&self, client: &Client) -> Result<Option<ServerStatus>> {
        let request = tonic::Request::new(StatusRequest {
            client_address: Some(Address {
                hostname: client.address.ip().to_string(),
                port: u32::from(client.address.port()),
            }),
            server_address: Some(Address {
                hostname: client.server_address.to_string(),
                port: u32::from(client.server_port),
            }),
            protocol: client.protocol_version as u64,
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
    async fn status(&self, client: &Client) -> Result<Option<ServerStatus>> {
        let start = Instant::now();
        let status = self.status(client).await;
        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        status
    }
}
