use crate::proto::authentication_client::AuthenticationClient;
use crate::proto::{AuthenticationRequest, authentication_response};
use passage_adapters::authentication::{AuthenticationAdapter, Profile};
use passage_adapters::{Client, Error, Player, metrics, reject, reject_reason};
use std::fmt::{Debug, Formatter};
use tokio::time::Instant;
use tonic::transport::Channel;
use tracing::instrument;

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "grpc_authentication_adapter";

pub struct GrpcAuthenticationAdapter {
    client: AuthenticationClient<Channel>,
}

impl Debug for GrpcAuthenticationAdapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", ADAPTER_TYPE)
    }
}

impl GrpcAuthenticationAdapter {
    pub async fn new<D>(address: D) -> Result<Self, Error>
    where
        D: TryInto<tonic::transport::Endpoint>,
        D::Error: Into<tonic::codegen::StdError>,
    {
        Ok(Self {
            client: AuthenticationClient::connect(address)
                .await
                .map_err(|err| Error::FailedInitialization {
                    adapter_type: ADAPTER_TYPE,
                    cause: err.into(),
                })?,
        })
    }

    #[instrument(skip_all)]
    async fn authenticate(
        &self,
        client: &Client,
        player: &Player,
        shared_secret: &[u8],
        encoded_public: &[u8],
    ) -> Result<Profile, Error> {
        let request = tonic::Request::new(AuthenticationRequest {
            client: Some(client.clone().into()),
            player: Some(player.clone().into()),
            shared_secret: shared_secret.to_vec(),
            encoded_public: encoded_public.to_vec(),
        });
        let response = self
            .client
            .clone()
            .authenticate(request)
            .await
            .map_err(|err| Error::FailedFetch {
                adapter_type: ADAPTER_TYPE,
                cause: err.into(),
            })?;

        // return the result right away
        match response.into_inner().reason {
            None => Err(reject(ADAPTER_TYPE)),
            Some(authentication_response::Reason::Key(key)) => {
                Err(reject_reason(ADAPTER_TYPE, key))
            }
            Some(authentication_response::Reason::Profile(profile)) => Ok(profile.try_into()?),
        }
    }
}

impl AuthenticationAdapter for GrpcAuthenticationAdapter {
    #[instrument(skip_all)]
    async fn authenticate(
        &self,
        client: &Client,
        player: &Player,
        shared_secret: &[u8],
        encoded_public: &[u8],
    ) -> Result<Profile, Error> {
        let start = Instant::now();
        let profile = self
            .authenticate(client, player, shared_secret, encoded_public)
            .await;
        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        profile
    }
}
