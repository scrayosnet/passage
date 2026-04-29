use crate::DnsError;
use hickory_resolver::Resolver;
use hickory_resolver::config::ResolverConfig;
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::proto::rr::RData::SRV;
use passage_adapters::discovery::DiscoveryAdapter;
use passage_adapters::{Client, Target, metrics};
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

/// The name of the adapter. It is primarily used for logging and metrics.
const ADAPTER_TYPE: &str = "dns_discovery_adapter";

/// The type of DNS record to query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordType {
    /// SRV records for service discovery (includes port in DNS response).
    Srv,
    /// A/AAAA records (requires default port to be specified).
    A {
        /// The port to use for A/AAAA records.
        port: u16,
    },
}

/// DNS-based discovery adapter that resolves targets from DNS records.
pub struct DnsDiscoveryAdapter {
    inner: Arc<RwLock<Vec<Target>>>,
    token: CancellationToken,
}

impl Debug for DnsDiscoveryAdapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "DnsDiscoveryAdapter")
    }
}

impl DnsDiscoveryAdapter {
    /// Creates a new DNS discovery adapter.
    ///
    /// # Arguments
    ///
    /// * `domain` - The DNS domain to query
    /// * `refresh_duration` - How often to re-query DNS in seconds
    /// * `record_type` - The type of DNS record to query
    pub async fn new(
        domain: String,
        refresh_duration: u64,
        record_type: RecordType,
    ) -> Result<Self, DnsError> {
        let refresh_interval = Duration::from_secs(refresh_duration);
        let inner: Arc<RwLock<Vec<Target>>> = Arc::new(RwLock::new(Vec::new()));
        let token = CancellationToken::new();

        // create DNS resolver
        let resolver = Resolver::builder_with_config(
            ResolverConfig::default(),
            TokioRuntimeProvider::default(),
        )
        .build()
        .map_err(|err| DnsError::BuildFailed { cause: err })?;

        // start background refresh task
        let _inner = Arc::clone(&inner);
        let _token = token.clone();
        let _domain = domain.clone();
        let mut interval = tokio::time::interval(refresh_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        tokio::spawn(async move {
            info!("starting DNS discovery watcher");
            loop {
                tokio::select! {
                    biased;
                    _ = _token.cancelled() => break,
                    _ = interval.tick() => {},
                }

                // query DNS based on record type
                let targets = match record_type {
                    RecordType::Srv => Self::query_srv(&resolver, &_domain).await,
                    RecordType::A { port } => Self::query_a(&resolver, &_domain, port).await,
                };

                match targets {
                    Ok(new_targets) => {
                        debug!(count = new_targets.len(), "discovered targets from DNS");
                        let mut inner = _inner.write().await;
                        *inner = new_targets;
                    }
                    Err(err) => {
                        warn!(err = ?err, "failed to query DNS");
                    }
                }
            }
            info!("stopping DNS discovery watcher");
        });

        Ok(Self { inner, token })
    }

    /// Queries SRV records and returns targets.
    async fn query_srv(
        resolver: &Resolver<TokioRuntimeProvider>,
        domain: &str,
    ) -> Result<Vec<Target>, DnsError> {
        let response = resolver
            .srv_lookup(domain)
            .await
            .map_err(|err| DnsError::LookupFailed { cause: err })?;

        let mut targets = Vec::new();

        for srv in response.answers() {
            let SRV(srv) = &srv.data else { continue };
            let target_name = srv.target.to_utf8();
            let port = srv.port;

            // Resolve the target hostname to IP addresses
            let lookup = resolver
                .lookup_ip(target_name.as_str())
                .await
                .map_err(|err| DnsError::LookupFailed { cause: err })?;

            for ip in lookup.iter() {
                let address = SocketAddr::new(ip, port);
                let identifier = format!("{}:{}", target_name, port);

                targets.push(Target {
                    identifier: identifier.clone(),
                    address,
                    priority: 0,
                    meta: HashMap::from([
                        ("priority".to_string(), srv.priority.to_string()),
                        ("weight".to_string(), srv.weight.to_string()),
                    ]),
                });
            }
        }

        Ok(targets)
    }

    /// Queries A/AAAA records and returns targets.
    async fn query_a(
        resolver: &Resolver<TokioRuntimeProvider>,
        domain: &str,
        port: u16,
    ) -> Result<Vec<Target>, DnsError> {
        let lookup = resolver
            .lookup_ip(domain)
            .await
            .map_err(|err| DnsError::LookupFailed { cause: err })?;

        let mut targets = Vec::new();

        for ip in lookup.iter() {
            let address = SocketAddr::new(ip, port);
            let identifier = format!("{}:{}", domain, port);

            targets.push(Target {
                identifier: identifier.clone(),
                address,
                priority: 0,
                meta: HashMap::from([]),
            });
        }

        Ok(targets)
    }
}

impl Drop for DnsDiscoveryAdapter {
    fn drop(&mut self) {
        self.token.cancel();
    }
}

impl DiscoveryAdapter for DnsDiscoveryAdapter {
    async fn discover(&self, _client: &Client) -> passage_adapters::Result<Vec<Target>> {
        let start = Instant::now();
        let servers = self.inner.read().await.clone();
        metrics::adapter_duration::record(ADAPTER_TYPE, start);
        Ok(servers)
    }
}
