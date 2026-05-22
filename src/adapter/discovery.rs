use crate::adapter::{opt_to_regex, opt_vec_to_uuid};
use crate::config;
use crate::config::DnsDiscoveryRecordType;
use passage_adapters::discovery_action::meta_filter::{FilterOperation, FilterRule};
use passage_adapters::{
    Client, DiscoveryActionAdapter, FixedDiscoveryAdapter, MetaFilterAdapter, Player,
    PlayerAllowFilterAdapter, PlayerBlockFilterAdapter, PlayerFillStrategyAdapter, Target,
};
#[cfg(feature = "adapters-agones")]
use passage_adapters_agones::AgonesDiscoveryAdapter;
use passage_adapters_agones::AgonesDiscoveryAdapterConfig;
#[cfg(feature = "adapters-dns")]
use passage_adapters_dns::{DnsDiscoveryAdapter, RecordType};
#[cfg(feature = "adapters-grpc")]
use passage_adapters_grpc::GrpcDiscoveryActionAdapter;
#[cfg(feature = "adapters-grpc")]
use passage_adapters_grpc::GrpcDiscoveryAdapter;
use std::fmt::{Display, Formatter};

/// Runtime-selected discovery action adapter.
///
/// Wraps all built-in and feature-gated discovery sources and filters behind a single enum, used
/// as elements in the per-route discovery pipeline.
#[derive(Debug)]
pub enum DynDiscoveryActionAdapter {
    /// Returns a static pre-configured list of targets.
    FixedDiscovery(FixedDiscoveryAdapter),
    /// Allocates a target from an Agones `GameServer` fleet.
    #[cfg(feature = "adapters-agones")]
    AgonesDiscovery(AgonesDiscoveryAdapter),
    /// Fetches targets from an external gRPC discovery service.
    #[cfg(feature = "adapters-grpc")]
    GrpcDiscovery(GrpcDiscoveryAdapter),
    /// Resolves targets from DNS SRV or A records.
    #[cfg(feature = "adapters-dns")]
    DnsDiscovery(DnsDiscoveryAdapter),
    /// Delegates the entire pipeline step to an external gRPC service.
    #[cfg(feature = "adapters-grpc")]
    Grpc(GrpcDiscoveryActionAdapter),
    /// Removes targets whose metadata does not match configured rules.
    MetaFilter(MetaFilterAdapter),
    /// Removes all targets when the player is not on the allow-list.
    PlayerAllowFilter(PlayerAllowFilterAdapter),
    /// Removes all targets when the player is on the block-list.
    PlayerBlockFilter(PlayerBlockFilterAdapter),
    /// Re-orders targets to maximise server utilisation.
    PlayerFillStrategy(PlayerFillStrategyAdapter),
}

impl Display for DynDiscoveryActionAdapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        pub use DynDiscoveryActionAdapter::*;
        match self {
            FixedDiscovery(_) => write!(f, "fixed-discovery"),
            #[cfg(feature = "adapters-agones")]
            AgonesDiscovery(_) => write!(f, "agones-discovery"),
            #[cfg(feature = "adapters-grpc")]
            GrpcDiscovery(_) => write!(f, "grpc-discovery"),
            #[cfg(feature = "adapters-dns")]
            DnsDiscovery(_) => write!(f, "dns-discovery"),
            #[cfg(feature = "adapters-grpc")]
            Grpc(_) => write!(f, "grpc-action"),
            MetaFilter(_) => write!(f, "meta-filter"),
            PlayerAllowFilter(_) => write!(f, "player-allow-filter"),
            PlayerBlockFilter(_) => write!(f, "player-block-filter"),
            PlayerFillStrategy(_) => write!(f, "player-fill-strategy"),
        }
    }
}

impl DiscoveryActionAdapter for DynDiscoveryActionAdapter {
    async fn apply(
        &self,
        client: &Client,
        player: &Player,
        targets: &mut Vec<Target>,
    ) -> passage_adapters::Result<()> {
        pub use DynDiscoveryActionAdapter::*;
        match self {
            FixedDiscovery(adapter) => adapter.apply(client, player, targets).await,
            #[cfg(feature = "adapters-agones")]
            AgonesDiscovery(adapter) => adapter.apply(client, player, targets).await,
            #[cfg(feature = "adapters-grpc")]
            GrpcDiscovery(adapter) => adapter.apply(client, player, targets).await,
            #[cfg(feature = "adapters-dns")]
            DnsDiscovery(adapter) => adapter.apply(client, player, targets).await,
            #[cfg(feature = "adapters-grpc")]
            Grpc(adapter) => adapter.apply(client, player, targets).await,
            MetaFilter(adapter) => adapter.apply(client, player, targets).await,
            PlayerAllowFilter(adapter) => adapter.apply(client, player, targets).await,
            PlayerBlockFilter(adapter) => adapter.apply(client, player, targets).await,
            PlayerFillStrategy(adapter) => adapter.apply(client, player, targets).await,
        }
    }
}

impl DynDiscoveryActionAdapter {
    /// Constructs the ordered adapter pipeline described by `config`.
    ///
    /// The first element is the discovery source; subsequent elements are action adapters applied
    /// in order. Returns all adapters as a `Vec` suitable for use with the blanket
    /// `DiscoveryActionAdapter for Vec<T>` implementation.
    pub async fn from_config(
        config: config::DiscoveryAdapter,
    ) -> Result<Vec<Self>, Box<dyn std::error::Error>> {
        let mut adapters = Vec::with_capacity(config.actions.len() + 1);
        adapters.push(Self::action_from_config(config.adapter).await?);
        for action in config.actions {
            adapters.push(Self::action_from_config(action).await?);
        }
        Ok(adapters)
    }

    async fn action_from_config(
        config: config::DiscoveryActionAdapter,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        use DynDiscoveryActionAdapter::*;
        use config::DiscoveryActionAdapter as conf;
        #[allow(unreachable_patterns)]
        match config {
            conf::FixedDiscovery(config) => {
                let adapter = FixedDiscoveryAdapter::new(config.targets);
                Ok(FixedDiscovery(adapter))
            }
            #[cfg(feature = "adapters-agones")]
            conf::AgonesDiscovery(config) => {
                let agones_config = AgonesDiscoveryAdapterConfig {
                    namespace: config.namespace,
                    selectors: config.selectors.into_iter().map(Into::into).collect(),
                    priorities: config.priorities.into_iter().map(Into::into).collect(),
                    scheduling: config.scheduling,
                    metadata: config.metadata.map(Into::into),
                    backoff: config.backoff,
                };
                let adapter = AgonesDiscoveryAdapter::new(agones_config).await?;
                Ok(AgonesDiscovery(adapter))
            }
            #[cfg(feature = "adapters-grpc")]
            conf::GrpcDiscovery(config) => {
                let adapter = GrpcDiscoveryAdapter::new(config.address).await?;
                Ok(GrpcDiscovery(adapter))
            }
            #[cfg(feature = "adapters-dns")]
            conf::DnsDiscovery(config) => {
                let record_type = match config.record_type {
                    DnsDiscoveryRecordType::Srv => RecordType::Srv,
                    DnsDiscoveryRecordType::A(conf) => RecordType::A { port: conf.port },
                };
                let adapter =
                    DnsDiscoveryAdapter::new(config.domain, config.refresh_interval, record_type)
                        .await?;
                Ok(DnsDiscovery(adapter))
            }
            #[cfg(feature = "adapters-grpc")]
            conf::Grpc(config) => {
                let adapter = GrpcDiscoveryActionAdapter::new(config.address).await?;
                Ok(Grpc(adapter))
            }
            conf::MetaFilter(config) => {
                let adapter =
                    MetaFilterAdapter::new(config.rules.into_iter().map(Into::into).collect());
                Ok(MetaFilter(adapter))
            }
            conf::PlayerAllowFilter(config) => {
                let adapter = PlayerAllowFilterAdapter::new(
                    config.usernames,
                    opt_to_regex(config.username)?,
                    opt_vec_to_uuid(config.ids)?,
                );
                Ok(PlayerAllowFilter(adapter))
            }
            conf::PlayerBlockFilter(config) => {
                let adapter = PlayerBlockFilterAdapter::new(
                    config.usernames,
                    opt_to_regex(config.username)?,
                    opt_vec_to_uuid(config.ids)?,
                );
                Ok(PlayerBlockFilter(adapter))
            }
            conf::PlayerFillStrategy(config) => {
                let adapter = PlayerFillStrategyAdapter::new(config.field, config.max_players);
                Ok(PlayerFillStrategy(adapter))
            }
            _ => Err("unknown discovery adapter configured".into()),
        }
    }
}

impl From<config::FilterRule> for FilterRule {
    fn from(value: config::FilterRule) -> Self {
        Self {
            key: value.key,
            operation: value.operation.into(),
        }
    }
}

impl From<config::FilterOperation> for FilterOperation {
    fn from(value: config::FilterOperation) -> Self {
        match value {
            config::FilterOperation::Equals(value) => FilterOperation::Equals(value),
            config::FilterOperation::NotEquals(value) => FilterOperation::NotEquals(value),
            config::FilterOperation::Exists => FilterOperation::Exists,
            config::FilterOperation::NotExists => FilterOperation::NotExists,
            config::FilterOperation::In(values) => FilterOperation::In(values),
            config::FilterOperation::NotIn(values) => FilterOperation::NotIn(values),
        }
    }
}
