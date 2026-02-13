use crate::connection::{Connection, Error};
use crate::helper::rate_limiter::RateLimiter;
use crate::localization::Localization;
use crate::metrics;
use crate::mojang::Mojang;
use passage_adapters::discovery::DiscoveryAdapter;
use passage_adapters::status::StatusAdapter;
use passage_adapters::strategy::StrategyAdapter;
use proxy_header::ParseConfig;
use proxy_header::io::ProxiedStream;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};
use tokio::select;
use tokio::time::{Instant, timeout};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::{debug, info, instrument, warn};
use uuid::Uuid;

const DEFAULT_CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);

// the server listener
pub struct Listener<Disc, Stat, Stra, Api> {
    status_adapter: Arc<Stat>,
    discovery_adapter: Arc<Disc>,
    strategy_adapter: Arc<Stra>,
    mojang: Arc<Api>,
    localization: Arc<Localization>,
    tracker: TaskTracker,
    rate_limiter: Option<RateLimiter<IpAddr>>,
    use_proxy_protocol: bool,
    connection_timeout: Duration,
    auth_secret: Option<Vec<u8>>,
}

impl<Disc, Stat, Stra, Api> Listener<Disc, Stat, Stra, Api>
where
    Disc: DiscoveryAdapter + 'static,
    Stat: StatusAdapter + 'static,
    Stra: StrategyAdapter + 'static,
    Api: Mojang + 'static,
{
    pub fn new(
        status_adapter: Arc<Stat>,
        discovery_adapter: Arc<Disc>,
        strategy_adapter: Arc<Stra>,
        mojang: Arc<Api>,
        localization: Arc<Localization>,
    ) -> Self {
        Self {
            discovery_adapter,
            status_adapter,
            strategy_adapter,
            mojang,
            localization,
            tracker: TaskTracker::new(),
            rate_limiter: None,
            use_proxy_protocol: false,
            connection_timeout: DEFAULT_CONNECTION_TIMEOUT,
            auth_secret: None,
        }
    }

    pub fn with_rate_limiter(mut self, rate_limiter: RateLimiter<IpAddr>) -> Self {
        self.rate_limiter = Some(rate_limiter);
        self
    }

    pub fn with_proxy_protocol(mut self, use_proxy_protocol: bool) -> Self {
        self.use_proxy_protocol = use_proxy_protocol;
        self
    }

    pub fn with_connection_timeout(mut self, connection_timeout: Duration) -> Self {
        self.connection_timeout = connection_timeout;
        self
    }

    pub fn with_auth_secret(mut self, auth_secret: Vec<u8>) -> Self {
        self.auth_secret = Some(auth_secret);
        self
    }

    pub fn with_auth_secret_opt(mut self, auth_secret: Option<Vec<u8>>) -> Self {
        self.auth_secret = auth_secret;
        self
    }

    #[instrument(skip_all)]
    pub async fn listen<A: ToSocketAddrs>(
        &mut self,
        address: A,
        stop: CancellationToken,
    ) -> Result<(), Box<dyn std::error::Error>> {
        info!(proxy = self.use_proxy_protocol, "starting listener");
        let listener = TcpListener::bind(address).await?;
        loop {
            // accept the next incoming connection
            let (stream, addr) = select! {
                accepted = listener.accept() => accepted?,
                _ = stop.cancelled() => {
                    info!("stopping listener");
                    break;
                },
            };

            // handle the connection
            let trace_id = Uuid::new_v4();
            self.handle(stream, addr, trace_id).await;
        }

        // wait for all connections to finish
        self.tracker.close();
        self.tracker.wait().await;

        info!("protocol server stopped successfully");
        Ok(())
    }

    #[instrument(skip(self, stream))]
    async fn handle(&mut self, stream: TcpStream, addr: SocketAddr, trace_id: Uuid) {
        let connection_start = Instant::now();

        // handle proxy protocol
        let proxy_config = ParseConfig {
            // not required for our use case
            include_tlvs: false,
            allow_v1: true,
            allow_v2: true,
        };

        let (mut stream, client_addr) = if self.use_proxy_protocol {
            match ProxiedStream::create_from_tokio(stream, proxy_config).await {
                Ok(stream) => {
                    let client_addr = stream
                        .proxy_header()
                        .proxied_address()
                        .map(|address| address.source)
                        .unwrap_or(addr);
                    (stream, client_addr)
                }
                Err(e) => {
                    debug!(
                        cause = e.to_string(),
                        addr = addr.to_string(),
                        "failed to parse proxy protocol header, connection closed"
                    );
                    return;
                }
            }
        } else {
            (ProxiedStream::unproxied(stream), addr)
        };
        debug!(addr = %client_addr, "handling new connection");

        // check rate limiter (use real client address)
        if let Some(rate_limiter) = &mut self.rate_limiter
            && !rate_limiter.enqueue(client_addr.ip())
        {
            info!(addr = client_addr.to_string(), "rate limited client");
            metrics::request_duration::record(connection_start, "rejected");

            if let Err(e) = stream.shutdown().await {
                debug!(cause = e.to_string(), "failed to close a client connection");
            }
            return;
        }

        let connection_timeout = self.connection_timeout;
        let status_adapter = self.status_adapter.clone();
        let discovery_adapter = self.discovery_adapter.clone();
        let strategy_adapter = self.strategy_adapter.clone();
        let mojang = self.mojang.clone();
        let localization = self.localization.clone();
        let auth_secret = self.auth_secret.clone();

        // create a new connection and run protocol
        self.tracker.spawn(async move {
            metrics::open_connections::inc();
            let mut connection = Connection::new(
                &mut stream,
                status_adapter,
                discovery_adapter,
                strategy_adapter,
                mojang,
                localization,
                auth_secret,
                client_addr,
            );

            // handle the client connection (ignore connection closed by the client)
            let timeout = timeout(connection_timeout, connection.listen()).await;
            let connection_result = match timeout {
                Ok(Err(Error::ConnectionClosed(_))) => "connection-closed",
                Ok(Err(err)) => {
                    warn!(cause = err.to_string(), "failed to handle connection");
                    err.as_label()
                }
                Ok(_) => "success",
                Err(_) => "timeout",
            };

            // flush connection and shutdown
            if let Err(err) = stream.shutdown().await {
                warn!(cause = err.to_string(), "failed to shutdown connection");
            }
            info!("closed connection");

            // update metrics
            metrics::request_duration::record(connection_start, connection_result);
            metrics::open_connections::dec();
        });
    }
}
