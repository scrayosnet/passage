use crate::config;
use passage_adapters::Protocol;
use passage_adapters::authentication::fixed::FixedAuthenticationAdapter;
use passage_adapters::authentication::{AuthenticationAdapter, Profile};
use passage_adapters_http::MojangAdapter;
use sentry::protocol::Uuid;
use std::net::SocketAddr;

#[derive(Debug)]
pub enum DynAuthenticationAdapter {
    Fixed(FixedAuthenticationAdapter),
    #[cfg(feature = "adapters-http")]
    Mojang(MojangAdapter),
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
        config: &config::TargetStrategy,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        match config.adapter.as_str() {
            "fixed" => {
                let Some(config) = config.fixed.clone() else {
                    return Err("fixed strategy adapter requires a configuration".into());
                };
                // TODO get profile from config
                let adapter = FixedAuthenticationAdapter::new();
                Ok(DynAuthenticationAdapter::Fixed(adapter))
            }
            #[cfg(feature = "adapters-http")]
            "mojang" => {
                let Some(config) = config.fixed.clone() else {
                    return Err("fixed strategy adapter requires a configuration".into());
                };
                // TODO get server id from config
                let adapter = MojangAdapter::default().with_server_id("".to_string());
                Ok(DynAuthenticationAdapter::Mojang(adapter))
            }
            _ => Err("unknown filter adapter configured".into()),
        }
    }
}
