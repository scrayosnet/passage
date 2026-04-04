use crate::proto::authentication_client::AuthenticationClient;
use crate::proto::{Address, AuthenticationRequest, authentication_response};
use passage_adapters::authentication::{AuthenticationAdapter, Profile};
use passage_adapters::{Error, Protocol, Reason, metrics};
use std::fmt::{Debug, Formatter};
use std::net::SocketAddr;
use tokio::time::Instant;
use tonic::transport::Channel;
use tracing::instrument;
use uuid::Uuid;

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
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        (user_name, user_id): (&str, &Uuid),
        shared_secret: &[u8],
        encoded_public: &[u8],
    ) -> Result<Reason<Profile>, Error> {
        let request = tonic::Request::new(AuthenticationRequest {
            client_address: Some(Address {
                hostname: client_addr.ip().to_string(),
                port: u32::from(client_addr.port()),
            }),
            server_address: Some(Address {
                hostname: server_addr.0.to_string(),
                port: u32::from(server_addr.1),
            }),
            protocol: protocol as u64,
            username: user_name.to_string(),
            user_id: user_id.to_string(),
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
            None => Ok(Reason::None(None)),
            Some(authentication_response::Reason::Key(key)) => Ok(Reason::None(Some(key))),
            Some(authentication_response::Reason::Profile(profile)) => {
                Ok(Reason::Some(profile.try_into()?))
            }
        }
    }
}

impl AuthenticationAdapter for GrpcAuthenticationAdapter {
    #[instrument(skip_all)]
    async fn authenticate(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        user: (&str, &Uuid),
        shared_secret: &[u8],
        encoded_public: &[u8],
    ) -> Result<Reason<Profile>, Error> {
        let start = Instant::now();
        let profile = self
            .authenticate(
                client_addr,
                server_addr,
                protocol,
                user,
                shared_secret,
                encoded_public,
            )
            .await;
        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        profile
    }
}
