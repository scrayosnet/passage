use crate::proto::TargetRequest;
use crate::proto::discovery_client::DiscoveryClient;
use passage_adapters::discovery::DiscoveryAdapter;
use passage_adapters::{Error, Target};
use std::fmt::{Debug, Formatter};
use tonic::transport::Channel;
use tracing::instrument;

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
                    adapter_type: "grpc_target_selection",
                    cause: err.into(),
                }
            })?,
        })
    }
}

impl DiscoveryAdapter for GrpcDiscoveryAdapter {
    #[instrument(skip_all)]
    async fn discover(&self) -> passage_adapters::Result<Vec<Target>> {
        let request = tonic::Request::new(TargetRequest {});
        let response = self
            .client
            .clone()
            .get_targets(request)
            .await
            .map_err(|err| Error::FailedFetch {
                adapter_type: "grpc_target_selection",
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
