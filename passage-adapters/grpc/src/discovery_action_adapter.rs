use crate::proto::discovery_action_client::DiscoveryActionClient;
use crate::proto::{ApplyRequest, Targets, apply_response};
use passage_adapters::discovery_action::DiscoveryActionAdapter;
use passage_adapters::{AdapterError, Client, Player, Target, metrics, reject_reason};
use std::fmt::{Debug, Formatter};
use tokio::time::Instant;
use tonic::transport::Channel;
use tracing::{Span, debug, field, instrument};

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "grpc_discovery_action_adapter";

/// The fully qualified gRPC service this adapter calls, as
/// [`rpc.service`](https://opentelemetry.io/docs/specs/semconv/rpc/rpc-spans/) wants it.
const RPC_SERVICE: &str = "scrayosnet.passage.adapter.DiscoveryAction";

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
    pub async fn new<D>(address: D) -> Result<Self, AdapterError>
    where
        D: TryInto<tonic::transport::Endpoint>,
        D::Error: Into<tonic::codegen::StdError>,
    {
        Ok(Self {
            client: DiscoveryActionClient::connect(address)
                .await
                .map_err(|err| AdapterError::FailedInitialization {
                    adapter_type: ADAPTER_TYPE,
                    cause: err.into(),
                })?,
        })
    }

    async fn apply(
        &self,
        client: &Client,
        player: &Player,
        targets: &mut Vec<Target>,
    ) -> Result<(), AdapterError> {
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
                .map_err(|err| AdapterError::FailedFetch {
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
    #[instrument(
        level = "info",
        name = "apply",
        skip_all,
        fields(
            otel.kind = "client",
            otel.name = "scrayosnet.passage.adapter.DiscoveryAction/Apply",
            rpc.system = "grpc",
            rpc.service = RPC_SERVICE,
            rpc.method = "Apply",
            adapter = ADAPTER_TYPE,
            targets.before = targets.len(),
            targets.after = field::Empty,
        ),
    )]
    async fn apply(
        &self,
        client: &Client,
        player: &Player,
        targets: &mut Vec<Target>,
    ) -> Result<(), AdapterError> {
        let start = Instant::now();
        let target = self.apply(client, player, targets).await;
        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        match &target {
            Ok(()) => {
                Span::current().record("targets.after", targets.len());
            }
            Err(err) => debug!(err = %err, "the discovery action service refused the request"),
        }
        target
    }
}
