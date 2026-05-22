use crate::proto::discovery_action_client::DiscoveryActionClient;
use crate::proto::{ApplyRequest, Targets, apply_response};
use passage_adapters::discovery_action::DiscoveryActionAdapter;
use passage_adapters::{Client, Error, Player, Target, metrics, reject_reason};
use std::fmt::{Debug, Formatter};
use tokio::time::Instant;
use tonic::transport::Channel;
use tracing::instrument;

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "grpc_discovery_action_adapter";

/// Discovery action adapter that delegates target filtering and selection to an external gRPC
/// service.
///
/// The service receives the current candidate list and returns either a modified list or a
/// rejection key to abort routing.
pub struct GrpcDiscoveryActionAdapter {
    /// The client by which requests are made.
    client: DiscoveryActionClient<Channel>,
}

impl Debug for GrpcDiscoveryActionAdapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", ADAPTER_TYPE)
    }
}

impl GrpcDiscoveryActionAdapter {
    /// Connects to the gRPC service at `address` and returns an initialized adapter.
    pub async fn new<D>(address: D) -> Result<Self, Error>
    where
        D: TryInto<tonic::transport::Endpoint>,
        D::Error: Into<tonic::codegen::StdError>,
    {
        Ok(Self {
            client: DiscoveryActionClient::connect(address)
                .await
                .map_err(|err| Error::FailedInitialization {
                    adapter_type: ADAPTER_TYPE,
                    cause: err.into(),
                })?,
        })
    }

    #[instrument(skip_all)]
    async fn apply(
        &self,
        client: &Client,
        player: &Player,
        targets: &mut Vec<Target>,
    ) -> Result<(), Error> {
        let request = tonic::Request::new(ApplyRequest {
            client: Some(client.clone().into()),
            player: Some(player.clone().into()),
            targets: targets.iter().map(Into::into).collect(),
        });
        let response =
            self.client
                .clone()
                .apply(request)
                .await
                .map_err(|err| Error::FailedFetch {
                    adapter_type: ADAPTER_TYPE,
                    cause: err.into(),
                })?;

        // return the result right away
        match response.into_inner().reason {
            // handle no response as noop
            None => Ok(()),
            Some(apply_response::Reason::Key(key)) => Err(reject_reason(ADAPTER_TYPE, key)),
            Some(apply_response::Reason::Targets(Targets { targets: new })) => {
                *targets = new
                    .into_iter()
                    .map(TryInto::try_into)
                    .collect::<Result<_, _>>()?;
                Ok(())
            }
        }
    }
}

impl DiscoveryActionAdapter for GrpcDiscoveryActionAdapter {
    #[instrument(skip_all)]
    async fn apply(
        &self,
        client: &Client,
        player: &Player,
        targets: &mut Vec<Target>,
    ) -> Result<(), Error> {
        let start = Instant::now();
        let target = self.apply(client, player, targets).await;
        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        target
    }
}
