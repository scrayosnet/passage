use crate::proto::LocalizationRequest;
use crate::proto::localization_client::LocalizationClient;
use passage_adapters::localization::LocalizationAdapter;
use passage_adapters::{AdapterError, metrics};
use std::fmt::{Debug, Formatter};
use tokio::time::Instant;
use tonic::transport::Channel;
use tracing::instrument;

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "grpc_localization_adapter";

/// The fully qualified gRPC service this adapter calls, as
/// [`rpc.service`](https://opentelemetry.io/docs/specs/semconv/rpc/rpc-spans/) wants it.
const RPC_SERVICE: &str = "scrayosnet.passage.adapter.Localization";

/// Localization adapter that resolves message keys via an external gRPC service.
pub struct GrpcLocalizationAdapter {
    /// The client by which requests are made.
    client: LocalizationClient<Channel>,
}

impl Debug for GrpcLocalizationAdapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", ADAPTER_TYPE)
    }
}

impl GrpcLocalizationAdapter {
    /// Connects to the gRPC service at `address` and returns an initialized adapter.
    pub async fn new<D>(address: D) -> Result<Self, AdapterError>
    where
        D: TryInto<tonic::transport::Endpoint>,
        D::Error: Into<tonic::codegen::StdError>,
    {
        Ok(Self {
            client: LocalizationClient::connect(address).await.map_err(|err| {
                AdapterError::FailedInitialization {
                    adapter_type: ADAPTER_TYPE,
                    cause: err.into(),
                }
            })?,
        })
    }

    async fn localize(
        &self,
        locale: Option<&str>,
        key: &str,
        params: &[(&'static str, String)],
    ) -> passage_adapters::Result<String> {
        let request = tonic::Request::new(LocalizationRequest {
            locale: locale.map(|locale| locale.to_string()),
            key: key.to_string(),
            params: params
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        });
        let response = self.client.clone().localize(request).await.map_err(|err| {
            AdapterError::FailedFetch {
                adapter_type: ADAPTER_TYPE,
                cause: err.into(),
            }
        })?;

        // return the result right away
        Ok(response.into_inner().message)
    }
}

impl LocalizationAdapter for GrpcLocalizationAdapter {
    #[instrument(
        level = "info",
        name = "localize",
        skip_all,
        fields(
            otel.kind = "client",
            otel.name = "scrayosnet.passage.adapter.Localization/Localize",
            rpc.system = "grpc",
            rpc.service = RPC_SERVICE,
            rpc.method = "Localize",
            adapter = ADAPTER_TYPE,
            locale = locale,
            key = key,
        ),
    )]
    async fn localize(
        &self,
        locale: Option<&str>,
        key: &str,
        params: &[(&'static str, String)],
    ) -> passage_adapters::Result<String> {
        let start = Instant::now();
        let message = self.localize(locale, key, params).await;
        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        message
    }
}
