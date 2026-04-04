use crate::proto::LocalizationRequest;
use crate::proto::localization_client::LocalizationClient;
use passage_adapters::localization::LocalizationAdapter;
use passage_adapters::{Error, metrics};
use std::fmt::{Debug, Formatter};
use tokio::time::Instant;
use tonic::transport::Channel;
use tracing::instrument;

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "grpc_localization_adapter";

pub struct GrpcLocalizationAdapter {
    client: LocalizationClient<Channel>,
}

impl Debug for GrpcLocalizationAdapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", ADAPTER_TYPE)
    }
}

impl GrpcLocalizationAdapter {
    pub async fn new<D>(address: D) -> Result<Self, Error>
    where
        D: TryInto<tonic::transport::Endpoint>,
        D::Error: Into<tonic::codegen::StdError>,
    {
        Ok(Self {
            client: LocalizationClient::connect(address).await.map_err(|err| {
                Error::FailedInitialization {
                    adapter_type: ADAPTER_TYPE,
                    cause: err.into(),
                }
            })?,
        })
    }

    #[instrument(skip_all)]
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
        let response =
            self.client
                .clone()
                .localize(request)
                .await
                .map_err(|err| Error::FailedFetch {
                    adapter_type: ADAPTER_TYPE,
                    cause: err.into(),
                })?;

        // return the result right away
        Ok(response.into_inner().message)
    }
}

impl LocalizationAdapter for GrpcLocalizationAdapter {
    #[instrument(skip_all)]
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
