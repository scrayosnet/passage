use crate::proto::TargetRequest;
use crate::proto::discovery_client::DiscoveryClient;
use passage_adapters::discovery::DiscoveryAdapter;
use passage_adapters::{Error, Result, Target, metrics};
use std::fmt::{Debug, Formatter};
use tokio::time::Instant;
use tonic::transport::Channel;
use tracing::instrument;

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "grpc_discovery_adapter";

pub struct GrpcDiscoveryAdapter {
    client: DiscoveryClient<Channel>,
}

impl Debug for GrpcDiscoveryAdapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "GrpcDiscoveryAdapter")
    }
}

impl GrpcDiscoveryAdapter {
    pub async fn new<D>(address: D) -> Result<Self, Error>
    where
        D: TryInto<tonic::transport::Endpoint>,
        D::Error: Into<tonic::codegen::StdError>,
    {
        Ok(Self {
            client: DiscoveryClient::connect(address).await.map_err(|err| {
                Error::FailedInitialization {
                    adapter_type: ADAPTER_TYPE,
                    cause: err.into(),
                }
            })?,
        })
    }

    #[instrument(skip_all)]
    async fn discover(&self) -> Result<Vec<Target>> {
        let start = Instant::now();
        let request = tonic::Request::new(TargetRequest {});
        let response = self
            .client
            .clone()
            .get_targets(request)
            .await
            .map_err(|err| Error::FailedFetch {
                adapter_type: ADAPTER_TYPE,
                cause: err.into(),
            })?;

        let targets = response
            .into_inner()
            .targets
            .into_iter()
            .map(TryInto::try_into)
            .collect::<_>();

        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        targets
    }
}

impl DiscoveryAdapter for GrpcDiscoveryAdapter {
    #[instrument(skip_all)]
    async fn discover(&self) -> Result<Vec<Target>> {
        let start = Instant::now();
        let targets = self.discover().await;
        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        targets
    }
}
