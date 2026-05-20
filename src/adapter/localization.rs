use crate::config;
use passage_adapters::FixedLocalizationAdapter;
use passage_adapters::localization::LocalizationAdapter;
use passage_adapters_grpc::GrpcLocalizationAdapter;
use std::fmt::{Display, Formatter};

/// Runtime-selected localization adapter.
///
/// Wraps every built-in and feature-gated [`LocalizationAdapter`] implementation behind a single
/// enum.
#[derive(Debug)]
pub enum DynLocalizationAdapter {
    /// Resolves messages from an in-memory translation map.
    Fixed(FixedLocalizationAdapter),
    /// Resolves messages via an external gRPC service.
    #[cfg(feature = "adapters-grpc")]
    Grpc(GrpcLocalizationAdapter),
}

impl Display for DynLocalizationAdapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fixed(_) => write!(f, "fixed"),
            #[cfg(feature = "adapters-grpc")]
            Self::Grpc(_) => write!(f, "grpc"),
        }
    }
}

impl LocalizationAdapter for DynLocalizationAdapter {
    async fn localize(
        &self,
        locale: Option<&str>,
        key: &str,
        params: &[(&'static str, String)],
    ) -> passage_adapters::Result<String> {
        match self {
            DynLocalizationAdapter::Fixed(adapter) => adapter.localize(locale, key, params).await,
            #[cfg(feature = "adapters-grpc")]
            DynLocalizationAdapter::Grpc(adapter) => adapter.localize(locale, key, params).await,
        }
    }
}

impl DynLocalizationAdapter {
    /// Constructs the adapter described by `config`, establishing any required connections.
    pub async fn from_config(
        config: config::LocalizationAdapter,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        #[allow(unreachable_patterns)]
        match config {
            config::LocalizationAdapter::Fixed(config) => {
                let adapter = FixedLocalizationAdapter::new(
                    config.default_locale,
                    config.messages,
                    config.warn_unknown_keys,
                );
                Ok(DynLocalizationAdapter::Fixed(adapter))
            }
            #[cfg(feature = "adapters-grpc")]
            config::LocalizationAdapter::Grpc(config) => {
                let adapter = GrpcLocalizationAdapter::new(config.address).await?;
                Ok(DynLocalizationAdapter::Grpc(adapter))
            }
            _ => Err("unknown localization adapter configured".into()),
        }
    }
}
