use passage_adapters::authentication::Profile;
use passage_adapters::{
    AuthenticationAdapter, Client, DiscoveryActionAdapter, LocalizationAdapter, Player, Result,
    ServerStatus, StatusAdapter, Target, reject_reason,
};
use regex::Regex;
use std::fmt::{Debug, Display, Formatter};
use std::sync::Arc;

pub type Routes<Stat, Disc, Auth, Loca> = Arc<[Arc<Route<Stat, Disc, Auth, Loca>>]>;

#[derive(Clone, Debug)]
pub struct Route<Stat, Disc, Auth, Loca> {
    pub hostname: Regex,
    pub status_adapter: Stat,
    pub discovery_adapter: Disc,
    pub authentication_adapter: Auth,
    pub localization_adapter: Loca,
}

impl<Stat, Disc, Auth, Loca> Route<Stat, Disc, Auth, Loca>
where
    Stat: StatusAdapter,
    Disc: DiscoveryActionAdapter,
    Auth: AuthenticationAdapter,
    Loca: LocalizationAdapter,
{
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
