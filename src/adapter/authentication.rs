use crate::config;
use passage_adapters::authentication::fixed::FixedAuthenticationAdapter;
use passage_adapters::authentication::{AuthenticationAdapter, Profile};
use passage_adapters::{Client, DisabledAuthenticationAdapter, Player};
use passage_adapters_grpc::authentication_adapter::GrpcAuthenticationAdapter;
use passage_adapters_http::MojangAdapter;
use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum DynAuthenticationAdapter {
    Disabled(DisabledAuthenticationAdapter),
    Fixed(FixedAuthenticationAdapter),
    #[cfg(feature = "adapters-grpc")]
    Grpc(GrpcAuthenticationAdapter),
    #[cfg(feature = "adapters-http")]
    Mojang(MojangAdapter),
}

impl Display for DynAuthenticationAdapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled(_) => write!(f, "disabled"),
            Self::Fixed(_) => write!(f, "fixed"),
            #[cfg(feature = "adapters-grpc")]
            Self::Grpc(_) => write!(f, "grpc"),
            #[cfg(feature = "adapters-http")]
            Self::Mojang(_) => write!(f, "mojang"),
        }
    }
}

impl AuthenticationAdapter for DynAuthenticationAdapter {
    async fn authenticate(
        &self,
        client: &Client,
        player: &Player,
        shared_secret: &[u8],
        encoded_public: &[u8],
    ) -> passage_adapters::Result<Profile> {
        match self {
            DynAuthenticationAdapter::Disabled(adapter) => {
                adapter
                    .authenticate(client, player, shared_secret, encoded_public)
                    .await
            }
            DynAuthenticationAdapter::Fixed(adapter) => {
                adapter
                    .authenticate(client, player, shared_secret, encoded_public)
                    .await
            }
            #[cfg(feature = "adapters-grpc")]
            DynAuthenticationAdapter::Grpc(adapter) => {
                adapter
                    .authenticate(client, player, shared_secret, encoded_public)
                    .await
            }
            #[cfg(feature = "adapters-http")]
            DynAuthenticationAdapter::Mojang(adapter) => {
                adapter
                    .authenticate(client, player, shared_secret, encoded_public)
                    .await
            }
        }
    }
}

impl DynAuthenticationAdapter {
    pub async fn from_config(
        config: config::AuthenticationAdapter,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        #[allow(unreachable_patterns)]
        match config {
            config::AuthenticationAdapter::Disabled => {
                let adapter = DisabledAuthenticationAdapter::new();
                Ok(DynAuthenticationAdapter::Disabled(adapter))
            }
            config::AuthenticationAdapter::Fixed(config) => {
                let adapter = FixedAuthenticationAdapter::new(config.profile);
                Ok(DynAuthenticationAdapter::Fixed(adapter))
            }
            #[cfg(feature = "adapters-grpc")]
            config::AuthenticationAdapter::Grpc(config) => {
                let adapter = GrpcAuthenticationAdapter::new(config.address).await?;
                Ok(DynAuthenticationAdapter::Grpc(adapter))
            }
            #[cfg(feature = "adapters-http")]
            config::AuthenticationAdapter::Mojang(config) => {
                let adapter = MojangAdapter::default().with_server_id(config.server_id);
                Ok(DynAuthenticationAdapter::Mojang(adapter))
            }
            _ => Err("unknown authentication adapter configured".into()),
        }
    }
}
