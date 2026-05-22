use passage_adapters::authentication::Profile;
use passage_adapters::{
    AuthenticationAdapter, Client, DiscoveryActionAdapter, LocalizationAdapter, Player, Result,
    ServerStatus, StatusAdapter, Target, reject_reason,
};
use regex::Regex;
use std::fmt::{Debug, Display, Formatter};
use std::sync::Arc;

/// A shared, immutable slice of routes. The inner `Arc` allows individual routes to be cloned
/// cheaply across connections.
pub type Routes<Stat, Disc, Auth, Loca> = Arc<[Arc<Route<Stat, Disc, Auth, Loca>>]>;

/// A virtual-host routing rule that ties a hostname regex to a set of adapters.
///
/// Incoming connections are matched against [`Route::hostname`]; the first matching route is
/// selected. The route then acts as the single adapter entry-point for the connection, delegating
/// to each inner adapter in turn.
#[derive(Clone, Debug)]
pub struct Route<Stat, Disc, Auth, Loca> {
    /// Regular expression matched against the server address the client sent in the handshake.
    pub hostname: Regex,
    /// Adapter used to answer status ping requests for this route.
    pub status_adapter: Stat,
    /// Adapter pipeline used to discover and select a backend target for this route.
    pub discovery_adapter: Disc,
    /// Adapter used to authenticate the connecting player for this route.
    pub authentication_adapter: Auth,
    /// Adapter used to resolve localised messages for this route.
    pub localization_adapter: Loca,
}

impl<Stat, Disc, Auth, Loca> Route<Stat, Disc, Auth, Loca>
where
    Stat: StatusAdapter,
    Disc: DiscoveryActionAdapter,
    Auth: AuthenticationAdapter,
    Loca: LocalizationAdapter,
{
    /// Runs the full discovery pipeline and returns the single selected [`Target`].
    ///
    /// Returns `Err` if the pipeline produces no candidates.
    pub async fn select(&self, client: &Client, player: &Player) -> Result<Target> {
        let mut targets = Vec::new();
        self.discovery_adapter
            .apply(client, player, &mut targets)
            .await?;
        targets
            .pop()
            .ok_or_else(|| reject_reason("adapters", "disconnect_no_target"))
    }
}

impl<Stat, Disc, Auth, Loca> Display for Route<Stat, Disc, Auth, Loca>
where
    Stat: Debug,
    Disc: Debug,
    Auth: Debug,
    Loca: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Route")
            .field("hostname", &self.hostname)
            .field("status_adapter", &self.status_adapter)
            .field("discovery_adapter", &self.discovery_adapter)
            .field("authentication_adapter", &self.authentication_adapter)
            .field("localization_adapter", &self.localization_adapter)
            .finish()
    }
}

impl<Stat, Disc, Auth, Loca> StatusAdapter for Route<Stat, Disc, Auth, Loca>
where
    Stat: StatusAdapter,
    Disc: DiscoveryActionAdapter,
    Auth: AuthenticationAdapter,
    Loca: LocalizationAdapter,
{
    async fn status(&self, client: &Client) -> Result<Option<ServerStatus>> {
        self.status_adapter.status(client).await
    }
}

impl<Stat, Disc, Auth, Loca> DiscoveryActionAdapter for Route<Stat, Disc, Auth, Loca>
where
    Stat: StatusAdapter,
    Disc: DiscoveryActionAdapter,
    Auth: AuthenticationAdapter,
    Loca: LocalizationAdapter,
{
    async fn apply(
        &self,
        client: &Client,
        player: &Player,
        targets: &mut Vec<Target>,
    ) -> Result<()> {
        self.discovery_adapter.apply(client, player, targets).await
    }
}

impl<Stat, Disc, Auth, Loca> AuthenticationAdapter for Route<Stat, Disc, Auth, Loca>
where
    Stat: StatusAdapter,
    Disc: DiscoveryActionAdapter,
    Auth: AuthenticationAdapter,
    Loca: LocalizationAdapter,
{
    async fn authenticate(
        &self,
        client: &Client,
        player: &Player,
        shared_secret: &[u8],
        encoded_public: &[u8],
    ) -> Result<Profile> {
        self.authentication_adapter
            .authenticate(client, player, shared_secret, encoded_public)
            .await
    }
}

impl<Stat, Disc, Auth, Loca> LocalizationAdapter for Route<Stat, Disc, Auth, Loca>
where
    Stat: StatusAdapter,
    Disc: DiscoveryActionAdapter,
    Auth: AuthenticationAdapter,
    Loca: LocalizationAdapter,
{
    async fn localize(
        &self,
        locale: Option<&str>,
        key: &str,
        params: &[(&'static str, String)],
    ) -> Result<String> {
        self.localization_adapter
            .localize(locale, key, params)
            .await
    }
}
