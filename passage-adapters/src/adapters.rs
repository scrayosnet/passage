use crate::authentication::{AuthenticationAdapter, Profile};
use crate::discovery::DiscoveryAdapter;
use crate::filter::FilterAdapter;
use crate::localization::LocalizationAdapter;
use crate::status::StatusAdapter;
use crate::strategy::StrategyAdapter;
use crate::{Protocol, Result, ServerStatus, Target};
use std::fmt::{Debug, Display, Formatter};
use std::net::SocketAddr;
use uuid::Uuid;

pub struct Adapters<Stat, Disc, Filt, Stra, Auth, Loca> {
    status_adapter: Stat,
    discovery_adapter: Disc,
    filter_adapter: Filt,
    strategy_adapter: Stra,
    authentication_adapter: Auth,
    localization_adapter: Loca,
}

impl<Stat, Disc, Filt, Stra, Auth, Loca> Adapters<Stat, Disc, Filt, Stra, Auth, Loca>
where
    Stat: StatusAdapter,
    Disc: DiscoveryAdapter,
    Filt: FilterAdapter,
    Stra: StrategyAdapter,
    Auth: AuthenticationAdapter,
    Loca: LocalizationAdapter,
{
    pub fn new(
        status_adapter: Stat,
        discovery_adapter: Disc,
        filter_adapter: Filt,
        strategy_adapter: Stra,
        authentication_adapter: Auth,
        localization_adapter: Loca,
    ) -> Self {
        Self {
            status_adapter,
            discovery_adapter,
            filter_adapter,
            strategy_adapter,
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

    pub fn filter_adapter(&self) -> &Filt {
        &self.filter_adapter
    }

    pub fn strategy_adapter(&self) -> &Stra {
        &self.strategy_adapter
    }

    pub fn authentication_adapter(&self) -> &Auth {
        &self.authentication_adapter
    }

    pub fn localization_adapter(&self) -> &Loca {
        &self.localization_adapter
    }

    pub async fn select(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        user: (&str, &Uuid),
    ) -> Result<Option<Target>> {
        let all_targets = self.discover().await?;
        let filtered_targets = self
            .filter(client_addr, server_addr, protocol, user, all_targets)
            .await?;
        let selected_target = self
            .strategize(client_addr, server_addr, protocol, user, filtered_targets)
            .await?;
        Ok(selected_target)
    }
}

impl<Stat, Disc, Filt, Stra, Auth, Loca> Debug for Adapters<Stat, Disc, Filt, Stra, Auth, Loca>
where
    Stat: Debug,
    Disc: Debug,
    Filt: Debug,
    Stra: Debug,
    Auth: Debug,
    Loca: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Selector")
            .field("status_adapter", &self.status_adapter)
            .field("discovery_adapter", &self.discovery_adapter)
            .field("filter_adapter", &self.filter_adapter)
            .field("strategy_adapter", &self.strategy_adapter)
            .field("authentication_adapter", &self.authentication_adapter)
            .field("localization_adapter", &self.localization_adapter)
            .finish()
    }
}

impl<Stat, Disc, Filt, Stra, Auth, Loca> Display for Adapters<Stat, Disc, Filt, Stra, Auth, Loca>
where
    Stat: Display,
    Disc: Display,
    Filt: Display,
    Stra: Display,
    Auth: Display,
    Loca: Display,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Selector")
            .field("status_adapter", &self.status_adapter.to_string())
            .field("discovery_adapter", &self.discovery_adapter.to_string())
            .field("filter_adapter", &self.filter_adapter.to_string())
            .field("strategy_adapter", &self.strategy_adapter.to_string())
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

impl<Stat, Disc, Filt, Stra, Auth, Loca> StatusAdapter
    for Adapters<Stat, Disc, Filt, Stra, Auth, Loca>
where
    Stat: StatusAdapter,
    Disc: DiscoveryAdapter,
    Filt: FilterAdapter,
    Stra: StrategyAdapter,
    Auth: AuthenticationAdapter,
    Loca: LocalizationAdapter,
{
    async fn status(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
    ) -> Result<Option<ServerStatus>> {
        self.status_adapter
            .status(client_addr, server_addr, protocol)
            .await
    }
}

impl<Stat, Disc, Filt, Stra, Auth, Loca> DiscoveryAdapter
    for Adapters<Stat, Disc, Filt, Stra, Auth, Loca>
where
    Stat: StatusAdapter,
    Disc: DiscoveryAdapter,
    Filt: FilterAdapter,
    Stra: StrategyAdapter,
    Auth: AuthenticationAdapter,
    Loca: LocalizationAdapter,
{
    async fn discover(&self) -> Result<Vec<Target>> {
        self.discovery_adapter.discover().await
    }
}

impl<Stat, Disc, Filt, Stra, Auth, Loca> FilterAdapter
    for Adapters<Stat, Disc, Filt, Stra, Auth, Loca>
where
    Stat: StatusAdapter,
    Disc: DiscoveryAdapter,
    Filt: FilterAdapter,
    Stra: StrategyAdapter,
    Auth: AuthenticationAdapter,
    Loca: LocalizationAdapter,
{
    async fn filter(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        user: (&str, &Uuid),
        targets: Vec<Target>,
    ) -> Result<Vec<Target>> {
        self.filter_adapter
            .filter(client_addr, server_addr, protocol, user, targets)
            .await
    }
}

impl<Stat, Disc, Filt, Stra, Auth, Loca> StrategyAdapter
    for Adapters<Stat, Disc, Filt, Stra, Auth, Loca>
where
    Stat: StatusAdapter,
    Disc: DiscoveryAdapter,
    Filt: FilterAdapter,
    Stra: StrategyAdapter,
    Auth: AuthenticationAdapter,
    Loca: LocalizationAdapter,
{
    async fn strategize(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        user: (&str, &Uuid),
        targets: Vec<Target>,
    ) -> Result<Option<Target>> {
        self.strategy_adapter
            .strategize(client_addr, server_addr, protocol, user, targets)
            .await
    }
}

impl<Stat, Disc, Filt, Stra, Auth, Loca> AuthenticationAdapter
    for Adapters<Stat, Disc, Filt, Stra, Auth, Loca>
where
    Stat: StatusAdapter,
    Disc: DiscoveryAdapter,
    Filt: FilterAdapter,
    Stra: StrategyAdapter,
    Auth: AuthenticationAdapter,
    Loca: LocalizationAdapter,
{
    async fn authenticate(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        user: (&str, &Uuid),
        shared_secret: &[u8],
        encoded_public: &[u8],
    ) -> Result<Profile> {
        self.authentication_adapter
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

impl<Stat, Disc, Filt, Stra, Auth, Loca> LocalizationAdapter
    for Adapters<Stat, Disc, Filt, Stra, Auth, Loca>
where
    Stat: StatusAdapter,
    Disc: DiscoveryAdapter,
    Filt: FilterAdapter,
    Stra: StrategyAdapter,
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
