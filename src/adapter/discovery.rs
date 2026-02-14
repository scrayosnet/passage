use crate::config;
use passage_adapters::discovery::DiscoveryAdapter;
use passage_adapters::{FixedDiscoveryAdapter, Target};
#[cfg(feature = "adapters-agones")]
use passage_adapters_agones::{AgonesDiscoveryAdapter, watcher_config};
#[cfg(feature = "adapters-grpc")]
use passage_adapters_grpc::GrpcDiscoveryAdapter;

#[derive(Debug)]
pub enum DynDiscoveryAdapter {
    Fixed(FixedDiscoveryAdapter),
    #[cfg(feature = "adapters-agones")]
    Agones(AgonesDiscoveryAdapter),
    #[cfg(feature = "adapters-grpc")]
    Grpc(GrpcDiscoveryAdapter),
}

impl DiscoveryAdapter for DynDiscoveryAdapter {
    async fn discover(&self) -> passage_adapters::Result<Vec<Target>> {
        match self {
            DynDiscoveryAdapter::Fixed(adapter) => adapter.discover().await,
            #[cfg(feature = "adapters-agones")]
            DynDiscoveryAdapter::Agones(adapter) => adapter.discover().await,
            #[cfg(feature = "adapters-grpc")]
            DynDiscoveryAdapter::Grpc(adapter) => adapter.discover().await,
        }
    }
}

impl DynDiscoveryAdapter {
    pub async fn from_config(
        config: &config::TargetDiscovery,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        match config.adapter.as_str() {
            "fixed" => {
                let Some(config) = config.fixed.clone() else {
                    return Err("fixed discovery adapter requires a configuration".into());
                };
                let adapter = FixedDiscoveryAdapter::new(config.targets);
                Ok(DynDiscoveryAdapter::Fixed(adapter))
            }
            #[cfg(feature = "adapters-agones")]
            "agones" => {
                let Some(config) = config.agones.clone() else {
                    return Err("agones discovery adapter requires a configuration".into());
                };
                let watch = watcher_config::Config {
                    bookmarks: true,
                    label_selector: config.label_selector,
                    field_selector: None,
                    timeout: None,
                    list_semantic: watcher_config::ListSemantic::default(),
                    page_size: Some(500),
                    initial_list_strategy: watcher_config::InitialListStrategy::default(),
                };
                let adapter = AgonesDiscoveryAdapter::new(Some(&config.namespace), watch).await?;
                Ok(DynDiscoveryAdapter::Agones(adapter))
            }
            #[cfg(feature = "adapters-grpc")]
            "grpc" => {
                let Some(config) = config.grpc.clone() else {
                    return Err("grpc discovery adapter requires a configuration".into());
                };
                let adapter = GrpcDiscoveryAdapter::new(config.address).await?;
                Ok(DynDiscoveryAdapter::Grpc(adapter))
            }
            _ => Err("unknown discovery adapter configured".into()),
        }
    }
}
