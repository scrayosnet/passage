use crate::template::{Template, TemplateValues};
use crate::{GameServerAllocation, GameServerAllocationSpec};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::{Api, Client};
use opentelemetry::trace::TraceContextExt;
use passage_adapters::discovery::DiscoveryAdapter;
use passage_adapters::{Error, Target, metrics};
use serde_json::Value;
use std::fmt::{Debug, Formatter};
use tokio::time::Instant;
use tracing::warn;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "agones_discovery_adapter";

pub struct AgonesDiscoveryAdapterConfig {
    pub namespace: Option<String>,
    pub selectors: Vec<Template>,
    pub priorities: Vec<Template>,
    pub scheduling: Option<String>,
    pub metadata: Option<Template>,
}

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
    pub async fn new(config: AgonesDiscoveryAdapterConfig) -> Result<Self, Error> {
        // Build the client from the default config.
        let client = Client::try_default()
            .await
            .map_err(|err| Error::FailedInitialization {
                adapter_type: ADAPTER_TYPE,
                cause: err.into(),
            })?;

        // Build the client with the optional namespace.
        let api: Api<GameServerAllocation> = if let Some(namespace) = &config.namespace {
            Api::namespaced(client, namespace)
        } else {
            Api::all(client)
        };

        Ok(Self { config, api })
    }

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

        // Make the allocation request.
        let result = self
            .api
            .create(&kube::api::PostParams::default(), &allocation)
            .await
            .map_err(|err| Error::FailedFetch {
                adapter_type: ADAPTER_TYPE,
                cause: Box::new(err),
            })?; // TODO retry with backoff and jitter
        let Some(status) = &result.status else {
            warn!("Agones allocation returned no allocation status");
            return Ok(None);
        };

        // Convert the allocation (if any) into a target.
        let target = match status.state.as_deref() {
            Some("Allocated") => {
                let target = result.try_into().map_err(|err| Error::FailedParse {
                    adapter_type: ADAPTER_TYPE,
                    cause: Box::new(err),
                })?;
                Some(target)
            }
            Some("UnAllocated") => None, // TODO retry with backoff and jitter
            state => {
                warn!(state = ?state, "Agones allocation returned unsupported state");
                None
            }
        };
        Ok(target)
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
