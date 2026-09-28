use crate::proto::TargetRequest;
use crate::proto::discovery_client::DiscoveryClient;
use passage_adapters::discovery::DiscoveryAdapter;
use passage_adapters::{AdapterError, Client, Result, Target, metrics};
use std::fmt::{Debug, Formatter};
use tokio::time::Instant;
use tonic::transport::Channel;
use tracing::{Span, debug, field, instrument};

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "grpc_discovery_adapter";

/// The fully qualified gRPC service this adapter calls, as
/// [`rpc.service`](https://opentelemetry.io/docs/specs/semconv/rpc/rpc-spans/) wants it.
const RPC_SERVICE: &str = "scrayosnet.passage.adapter.Discovery";

/// Discovery adapter that fetches the available backend targets from an external gRPC service.
pub struct GrpcDiscoveryAdapter {
    /// The client by which requests are made.
    client: DiscoveryClient<Channel>,
}

impl Debug for GrpcDiscoveryAdapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "GrpcDiscoveryAdapter")
    }
}

impl GrpcDiscoveryAdapter {
    /// Connects to the gRPC service at `address` and returns an initialized adapter.
    pub async fn new<D>(address: D) -> Result<Self, AdapterError>
    where
        D: TryInto<tonic::transport::Endpoint>,
        D::Error: Into<tonic::codegen::StdError>,
    {
        Ok(Self {
            client: DiscoveryClient::connect(address).await.map_err(|err| {
                AdapterError::FailedInitialization {
                    adapter_type: ADAPTER_TYPE,
                    cause: err.into(),
                }
            })?,
        })
    }

    async fn discover(&self, client: &Client) -> Result<Vec<Target>> {
        let request = tonic::Request::new(TargetRequest {
            client: Some(client.clone().into()),
        });
        let response = self
            .client
            .clone()
            .get_targets(request)
            .await
            .map_err(|err| AdapterError::FailedFetch {
                adapter_type: ADAPTER_TYPE,
                cause: err.into(),
            })?;

        response
            .into_inner()
            .targets
            .into_iter()
            .map(TryInto::try_into)
            .collect::<_>()
    }
}

impl DiscoveryAdapter for GrpcDiscoveryAdapter {
    #[instrument(
        level = "info",
        name = "discover",
        skip_all,
        fields(
            otel.kind = "client",
            otel.name = "scrayosnet.passage.adapter.Discovery/GetTargets",
            rpc.system = "grpc",
            rpc.service = RPC_SERVICE,
            rpc.method = "GetTargets",
            adapter = ADAPTER_TYPE,
            targets = field::Empty,
        ),
    )]
    async fn discover(&self, client: &Client) -> Result<Vec<Target>> {
        let start = Instant::now();
        let targets = self.discover(client).await;
        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        match &targets {
            Ok(targets) => {
                Span::current().record("targets", targets.len());
            }
            Err(err) => debug!(err = %err, "the discovery service returned no targets"),
        }
        targets
    }
}
