use crate::authentication::{AuthenticationAdapter, Profile};
use crate::discovery_action::DiscoveryActionAdapter;
use crate::localization::LocalizationAdapter;
use crate::status::StatusAdapter;
use crate::{Client, Player, Result, ServerStatus, Target, reject_reason};
use std::fmt::{Debug, Display, Formatter};

pub struct Adapters<Stat, Disc, Auth, Loca> {
    status_adapter: Stat,
    discovery_adapter: Disc,
    authentication_adapter: Auth,
    localization_adapter: Loca,
}

impl<Stat, Disc, Auth, Loca> Adapters<Stat, Disc, Auth, Loca>
where
    Stat: StatusAdapter,
    Disc: DiscoveryActionAdapter,
    Auth: AuthenticationAdapter,
    Loca: LocalizationAdapter,
{
    pub fn new(
        status_adapter: Stat,
        discovery_adapter: Disc,
        authentication_adapter: Auth,
        localization_adapter: Loca,
    ) -> Self {
        Self {
            status_adapter,
            discovery_adapter,
            authentication_adapter,
            localization_adapter,
        }
    }

    pub fn status_adapter(&self) -> &Stat {
        &self.status_adapter
    }

    pub fn discovery_adapter(&self) -> &Disc {
        &self.discovery_adapter
    }

    pub fn authentication_adapter(&self) -> &Auth {
        &self.authentication_adapter
    }

    pub fn localization_adapter(&self) -> &Loca {
        &self.localization_adapter
    }

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

impl<Stat, Disc, Auth, Loca> Debug for Adapters<Stat, Disc, Auth, Loca>
where
    Stat: Debug,
    Disc: Debug,
    Auth: Debug,
    Loca: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Adapters")
            .field("status_adapter", &self.status_adapter)
            .field("discovery_adapter", &self.discovery_adapter)
            .field("authentication_adapter", &self.authentication_adapter)
            .field("localization_adapter", &self.localization_adapter)
            .finish()
    }
}

impl<Stat, Disc, Auth, Loca> Display for Adapters<Stat, Disc, Auth, Loca>
where
    Stat: Display,
    Disc: Display,
    Auth: Display,
    Loca: Display,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Selector")
            .field("status_adapter", &self.status_adapter.to_string())
            .field("discovery_adapter", &self.discovery_adapter.to_string())
            .field(
                "authentication_adapter",
                &self.authentication_adapter.to_string(),
            )
            .field(
                "localization_adapter",
                &self.localization_adapter.to_string(),
            )
            .finish()
    }
}

impl<Stat, Disc, Auth, Loca> StatusAdapter for Adapters<Stat, Disc, Auth, Loca>
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

impl<Stat, Disc, Auth, Loca> DiscoveryActionAdapter for Adapters<Stat, Disc, Auth, Loca>
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

impl<Stat, Disc, Auth, Loca> AuthenticationAdapter for Adapters<Stat, Disc, Auth, Loca>
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

impl<Stat, Disc, Auth, Loca> LocalizationAdapter for Adapters<Stat, Disc, Auth, Loca>
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
