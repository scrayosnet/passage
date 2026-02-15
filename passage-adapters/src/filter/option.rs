use crate::filter::FilterAdapter;
use crate::{Protocol, Target};
use regex::Regex;
use std::net::SocketAddr;
use uuid::Uuid;

#[derive(Debug, Default)]
pub struct OptionFilterAdapter<T> {
    hostname: Option<Regex>,
    filter: T,
}

impl<T> OptionFilterAdapter<T> {
    pub fn new(hostname: Option<String>, filter: T) -> crate::Result<Self> {
        let hostname = hostname.map(|s| Regex::new(&s)).transpose().map_err(|e| {
            crate::error::Error::FailedInitialization {
                adapter_type: "option_filter",
                cause: Box::new(e),
            }
        })?;
        Ok(Self { hostname, filter })
    }
}

impl<T> FilterAdapter for OptionFilterAdapter<T>
where
    T: FilterAdapter,
{
    async fn filter(
        &self,
        client_addr: &SocketAddr,
        server_addr: (&str, u16),
        protocol: Protocol,
        user: (&str, &Uuid),
        targets: Vec<Target>,
    ) -> crate::Result<Vec<Target>> {
        // check whether the hostname matches the regex
        if let Some(hostname) = &self.hostname
            && !hostname.is_match(server_addr.0)
        {
            return Ok(targets);
        }
        self.filter
            .filter(client_addr, server_addr, protocol, user, targets)
            .await
    }
}
