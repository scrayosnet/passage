use crate::proto::status_client::StatusClient;
use crate::proto::{Address, StatusRequest};
use passage_adapters::{
    AdapterError, Client, Result, ServerStatus, metrics, status::StatusAdapter,
};
use std::fmt::{Debug, Formatter};
use tokio::time::Instant;
use tonic::transport::Channel;
use tracing::{debug, instrument};

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "grpc_status_adapter";

/// The fully qualified gRPC service this adapter calls, as
/// [`rpc.service`](https://opentelemetry.io/docs/specs/semconv/rpc/rpc-spans/) wants it.
const RPC_SERVICE: &str = "scrayosnet.passage.adapter.Status";

/// Status adapter that retrieves server status from an external gRPC service.
pub struct GrpcStatusAdapter {
    /// The client by which requests are made.
    client: StatusClient<Channel>,
}

impl Debug for GrpcStatusAdapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "GrpcStatusAdapter")
    }
}

impl GrpcStatusAdapter {
    /// Connects to the gRPC service at `address` and returns an initialized adapter.
    pub async fn new<D>(address: D) -> Result<Self, AdapterError>
    where
        D: TryInto<tonic::transport::Endpoint>,
        D::Error: Into<tonic::codegen::StdError>,
    {
        Ok(Self {
            client: StatusClient::connect(address).await.map_err(|err| {
                AdapterError::FailedInitialization {
                    adapter_type: ADAPTER_TYPE,
                    cause: err.into(),
                }
            })?,
        })
    }

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
            protocol: client.protocol_version.get() as u64,
        });

        self.client
            .clone()
            .get_status(request)
            .await
            .map_err(|err| AdapterError::FailedFetch {
                adapter_type: ADAPTER_TYPE,
                cause: err.into(),
            })?
            .into_inner()
            .status
            .map(TryInto::try_into)
            .transpose()
    }
}

impl StatusAdapter for GrpcStatusAdapter {
    #[instrument(
        level = "info",
        name = "status",
        skip_all,
        fields(
            otel.kind = "client",
            otel.name = "scrayosnet.passage.adapter.Status/GetStatus",
            rpc.system = "grpc",
            rpc.service = RPC_SERVICE,
            rpc.method = "GetStatus",
            adapter = ADAPTER_TYPE,
        ),
    )]
    async fn status(&self, client: &Client) -> Result<Option<ServerStatus>> {
        let start = Instant::now();
        let status = self.status(client).await;
        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        if let Err(err) = &status {
            debug!(err = %err, "the status service did not return a status");
        }
        status
    }
}
