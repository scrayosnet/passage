use crate::config;
use passage_adapters::authentication::fixed::FixedAuthenticationAdapter;
use passage_adapters::authentication::{AuthenticationAdapter, Profile};
use passage_adapters::{DisabledAuthenticationAdapter, Protocol};
use passage_adapters_http::MojangAdapter;
use sentry::protocol::Uuid;
use std::fmt::{Display, Formatter};
use std::net::SocketAddr;

#[derive(Debug)]
pub enum DynAuthenticationAdapter {
    Disabled(DisabledAuthenticationAdapter),
    Fixed(FixedAuthenticationAdapter),
    #[cfg(feature = "adapters-http")]
    Mojang(MojangAdapter),
}

impl Display for DynAuthenticationAdapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled(_) => write!(f, "disabled"),
            Self::Fixed(_) => write!(f, "fixed"),
            #[cfg(feature = "adapters-http")]
            Self::Mojang(_) => write!(f, "mojang"),
        }
    }
}

impl AuthenticationAdapter for DynAuthenticationAdapter {
    async fn authenticate(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        user: (&str, &Uuid),
        shared_secret: &[u8],
        encoded_public: &[u8],
    ) -> passage_adapters::Result<Profile> {
        match self {
            DynAuthenticationAdapter::Disabled(adapter) => {
                adapter
                    .authenticate(
                        client_addr,
                        server_addr,
                        protocol,
                        user,
                        shared_secret,
                        encoded_public,
                    )
                    .await
            }
            DynAuthenticationAdapter::Fixed(adapter) => {
                adapter
                    .authenticate(
                        client_addr,
                        server_addr,
                        protocol,
                        user,
                        shared_secret,
                        encoded_public,
                    )
                    .await
            }
            #[cfg(feature = "adapters-http")]
            DynAuthenticationAdapter::Mojang(adapter) => {
                adapter
                    .authenticate(
                        client_addr,
                        server_addr,
                        protocol,
                        user,
                        shared_secret,
                        encoded_public,
                    )
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
            #[cfg(feature = "adapters-http")]
            config::AuthenticationAdapter::Mojang(config) => {
                let adapter = MojangAdapter::default().with_server_id(config.server_id);
                Ok(DynAuthenticationAdapter::Mojang(adapter))
            }
            _ => Err("unknown authentication adapter configured".into()),
        }
    }
}
