use crate::template::{Template, TemplateValues};
use crate::{GameServerAllocation, GameServerAllocationSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::{Api, Client};
use opentelemetry::trace::TraceContextExt;
use passage_adapters::backoff::ExponentialBackoff;
use passage_adapters::discovery::DiscoveryAdapter;
use passage_adapters::{Error, Target, metrics};
use serde_json::Value;
use std::fmt::{Debug, Formatter};
use std::time::Duration;
use tokio::time::Instant;
use tracing::{debug, warn};
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "agones_discovery_adapter";

/// Configuration for [`AgonesDiscoveryAdapter`].
#[derive(Debug, Clone, Default)]
pub struct AgonesDiscoveryAdapterConfig {
    /// Kubernetes namespace to search for `GameServer` resources. `None` searches all namespaces.
    pub namespace: Option<String>,
    /// Ordered list of `GameServerAllocation` selector templates used to find an eligible server.
    pub selectors: Vec<Template>,
    /// Priority list that influences allocation order within matching selectors.
    pub priorities: Vec<Template>,
    /// Agones scheduling strategy (`"Packed"` or `"Distributed"`).
    pub scheduling: Option<String>,
    /// Optional metadata template injected into the allocation request.
    pub metadata: Option<Template>,
    /// Back-off strategy for retrying unallocated responses.
    pub backoff: ExponentialBackoff,
}

/// Discovery adapter that allocates a `GameServer` from an [Agones](https://agones.dev) fleet.
///
/// Each call to [`DiscoveryAdapter::discover`] submits a `GameServerAllocation` to the Kubernetes
/// API. If the allocation returns `"UnAllocated"`, the adapter retries according to the configured
/// [`ExponentialBackoff`] strategy.
#[derive(Clone)]
pub struct AgonesDiscoveryAdapter {
    config: AgonesDiscoveryAdapterConfig,
    api: Api<GameServerAllocation>,
}

impl Debug for AgonesDiscoveryAdapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "AgonesDiscoveryAdapter")
    }
}

impl AgonesDiscoveryAdapter {
    /// Creates a new adapter using the provided Kubernetes `client`.
    pub async fn new_with_client(
        client: Client,
        config: AgonesDiscoveryAdapterConfig,
    ) -> Result<Self, Error> {
        // Build the client with the optional namespace.
        let api: Api<GameServerAllocation> = if let Some(namespace) = &config.namespace {
            Api::namespaced(client, namespace)
        } else {
            Api::all(client)
        };

        Ok(Self { config, api })
    }

    /// Creates a new adapter using the default in-cluster Kubernetes configuration.
    pub async fn new(config: AgonesDiscoveryAdapterConfig) -> Result<Self, Error> {
        // Build the client from the default config.
        let client = Client::try_default()
            .await
            .map_err(|err| Error::FailedInitialization {
                adapter_type: ADAPTER_TYPE,
                cause: err.into(),
            })?;
        Self::new_with_client(client, config).await
    }

    /// Submits a `GameServerAllocation` and returns the resulting [`Target`], retrying on
    /// `"UnAllocated"` responses up to the configured back-off limit.
    ///
    /// Returns `Ok(None)` if no server could be allocated within the attempt budget.
    pub async fn allocate(
        &self,
        client: &passage_adapters::Client,
    ) -> Result<Option<Target>, Error> {
        // Build the allocation request.
        let trace_id = tracing::Span::current()
            .context()
            .span()
            .span_context()
            .trace_id()
            .to_string();
        #[rustfmt::skip]
        let template_values = TemplateValues::from([
            ("{{ .Client.ProtocolVersion }}".to_string(), Value::String(client.protocol_version.to_string())),
            ("{{ .Client.ServerAddress }}".to_string(), Value::String(client.server_address.to_string())),
            ("{{ .Client.ServerPort }}".to_string(), Value::String(client.server_port.to_string())),
            ("{{ .Client.Address }}".to_string(), Value::String(client.address.to_string())),
            ("{{ .Request.TraceId }}".to_string(), Value::String(trace_id)),
        ]);
        let selectors = self
            .config
            .selectors
            .iter()
            .map(|selector| selector.template(&template_values))
            .collect();
        let priorities = self
            .config
            .priorities
            .iter()
            .map(|selector| selector.template(&template_values))
            .collect();
        let metadata = self
            .config
            .metadata
            .as_ref()
            .map(|selector| selector.template(&template_values));
        let allocation = GameServerAllocation {
            metadata: ObjectMeta::default(),
            spec: GameServerAllocationSpec {
                selectors: Some(selectors),
                priorities: Some(priorities),
                scheduling: self.config.scheduling.clone(),
                metadata,
            },
            status: None,
        };

        // Try to allocate a server with up to max attempts.
        for attempt in 1..(self.config.backoff.max_attempts + 1) {
            // Make the allocation request.
            let result = self
                .api
                .create(&kube::api::PostParams::default(), &allocation)
                .await
                .map_err(|err| Error::FailedFetch {
                    adapter_type: ADAPTER_TYPE,
                    cause: Box::new(err),
                })?;
            let Some(status) = &result.status else {
                warn!("agones allocation returned no allocation status");
                return Ok(None);
            };

            // Convert the allocation (if any) into a target.
            match status.state.as_deref() {
                Some("Allocated") => {
                    let target = result.try_into().map_err(|err| Error::FailedParse {
                        adapter_type: ADAPTER_TYPE,
                        cause: Box::new(err),
                    })?;
                    return Ok(Some(target));
                }
                Some("UnAllocated") => debug!("agones allocation returned unallocated, retrying"),
                state => warn!(state = ?state, "agones allocation returned unsupported state"),
            };

            // Wait for the next try.
            let Some(wait_secs) = self.config.backoff.secs_after(attempt).await else {
                break;
            };
            tokio::time::sleep(Duration::from_secs(wait_secs)).await;
        }

        warn!(
            attempts = self.config.backoff.max_attempts,
            "failed to allocate gameserver after max attempts"
        );
        Ok(None)
    }
}

impl DiscoveryAdapter for AgonesDiscoveryAdapter {
    async fn discover(
        &self,
        client: &passage_adapters::Client,
    ) -> passage_adapters::Result<Vec<Target>> {
        let start = Instant::now();
        // TODO handle errors with backoff
        let servers = self.allocate(client).await?.into_iter().collect();
        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        Ok(servers)
    }
}
