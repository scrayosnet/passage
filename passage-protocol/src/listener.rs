use crate::connection::{Connection, Error};
use crate::metrics;
use crate::rate_limiter::RateLimiter;
use passage_adapters::authentication::AuthenticationAdapter;
use passage_adapters::discovery::DiscoveryAdapter;
use passage_adapters::filter::FilterAdapter;
use passage_adapters::localization::LocalizationAdapter;
use passage_adapters::status::StatusAdapter;
use passage_adapters::strategy::StrategyAdapter;
pub use proxy_header::ParseConfig;
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

const DEFAULT_CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);

// the server listener
pub struct Listener<Stat, Disc, Filt, Stra, Auth, Loca> {
    status_adapter: Arc<Stat>,
    discovery_adapter: Arc<Disc>,
    filter_adapter: Arc<Filt>,
    strategy_adapter: Arc<Stra>,
    authentication_adapter: Arc<Auth>,
    localization_adapter: Arc<Loca>,
    tracker: TaskTracker,
    rate_limiter: Option<RateLimiter<IpAddr>>,
    proxy_protocol: Option<ParseConfig>,
    connection_timeout: Duration,
    auth_secret: Option<Vec<u8>>,
}

impl<Stat, Disc, Filt, Stra, Auth, Loca> Listener<Stat, Disc, Filt, Stra, Auth, Loca>
where
    Stat: StatusAdapter + 'static,
    Disc: DiscoveryAdapter + 'static,
    Filt: FilterAdapter + 'static,
    Stra: StrategyAdapter + 'static,
    Auth: AuthenticationAdapter + 'static,
    Loca: LocalizationAdapter + 'static,
{
    pub fn new(
        status_adapter: Arc<Stat>,
        discovery_adapter: Arc<Disc>,
        filter_adapter: Arc<Filt>,
        strategy_adapter: Arc<Stra>,
        authentication_adapter: Arc<Auth>,
        localization_adapter: Arc<Loca>,
    ) -> Self {
        Self {
            status_adapter,
            discovery_adapter,
            filter_adapter,
            strategy_adapter,
            authentication_adapter,
            localization_adapter,
            tracker: TaskTracker::new(),
            rate_limiter: None,
            proxy_protocol: None,
            connection_timeout: DEFAULT_CONNECTION_TIMEOUT,
            auth_secret: None,
        }
    }

    pub fn with_rate_limiter(mut self, rate_limiter: Option<RateLimiter<IpAddr>>) -> Self {
        self.rate_limiter = rate_limiter;
        self
    }

    pub fn with_proxy_protocol(mut self, proxy_protocol: Option<ParseConfig>) -> Self {
        self.proxy_protocol = proxy_protocol;
        self
    }

    pub fn with_connection_timeout(mut self, connection_timeout: Duration) -> Self {
        self.connection_timeout = connection_timeout;
        self
    }

    pub fn with_auth_secret(mut self, auth_secret: Option<Vec<u8>>) -> Self {
        self.auth_secret = auth_secret;
        self
    }

    #[instrument(skip_all)]
    pub async fn listen<A: ToSocketAddrs>(
        &mut self,
        address: A,
        stop: CancellationToken,
    ) -> Result<(), Box<dyn std::error::Error>> {
        info!(proxy = self.proxy_protocol.is_some(), "starting listener");
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
            self.handle(stream, addr).await;
        }

        // wait for all connections to finish
        self.tracker.close();
        self.tracker.wait().await;

        info!("protocol server stopped successfully");
        Ok(())
    }

    #[instrument(skip(self, stream))]
    async fn handle(&mut self, stream: TcpStream, addr: SocketAddr) {
        let connection_start = Instant::now();

        let (mut stream, client_addr) = if let Some(proxy_config) = self.proxy_protocol {
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
        let filter_adapter = self.filter_adapter.clone();
        let strategy_adapter = self.strategy_adapter.clone();
        let authentication_adapter = self.authentication_adapter.clone();
        let localization_adapter = self.localization_adapter.clone();
        let auth_secret = self.auth_secret.clone();

        // create a new connection and run protocol
        self.tracker.spawn(async move {
            metrics::open_connections::inc();
            let mut connection = Connection::new(
                &mut stream,
                status_adapter,
                discovery_adapter,
                filter_adapter,
                strategy_adapter,
                authentication_adapter,
                localization_adapter,
            )
            .with_client_address(client_addr)
            .with_auth_secret(auth_secret);

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
