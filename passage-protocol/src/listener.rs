use crate::config::Config;
use crate::connection::Connection;
use crate::rate_limiter::RateLimiter;
use crate::{Error, metrics};
use passage_adapters::authentication::AuthenticationAdapter;
use passage_adapters::localization::LocalizationAdapter;
use passage_adapters::status::StatusAdapter;
use passage_adapters::{Adapters, DiscoveryActionAdapter};
pub use proxy_header::ParseConfig;
use proxy_header::io::ProxiedStream;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};
use tokio::select;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing::{debug, info, instrument, warn};

// the server listener
pub struct Listener<Stat, Disc, Auth, Loca> {
    adapters: Arc<Adapters<Stat, Disc, Auth, Loca>>,
    tracker: TaskTracker,
    rate_limiter: Option<RateLimiter<IpAddr>>,
    config: Config,
}

impl<Stat, Disc, Auth, Loca> Listener<Stat, Disc, Auth, Loca>
where
    Stat: StatusAdapter + 'static,
    Disc: DiscoveryActionAdapter + 'static,
    Auth: AuthenticationAdapter + 'static,
    Loca: LocalizationAdapter + 'static,
{
    pub fn new(
        adapters: Arc<Adapters<Stat, Disc, Auth, Loca>>,
        rate_limiter: Option<RateLimiter<IpAddr>>,
        config: Config,
    ) -> Self {
        Self {
            adapters,
            tracker: TaskTracker::new(),
            rate_limiter,
            config,
        }
    }

    #[instrument(skip_all)]
    pub async fn listen<A: ToSocketAddrs>(
        &mut self,
        address: A,
        stop: CancellationToken,
    ) -> Result<(), Box<dyn std::error::Error>> {
        info!(
            proxy = self.config.proxy_protocol.is_some(),
            "starting listener"
        );
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
            self.handle(stream, addr, stop.child_token()).await;
        }

        // wait for all connections to finish
        self.tracker.close();
        self.tracker.wait().await;

        info!("protocol server stopped successfully");
        Ok(())
    }

    #[instrument(skip(self, stream))]
    async fn handle(&mut self, stream: TcpStream, addr: SocketAddr, stop: CancellationToken) {
        let connection_start = Instant::now();

        let (mut stream, client_addr) = if let Some(proxy_config) = &self.config.proxy_protocol {
            let proxy = ParseConfig {
                include_tlvs: false,
                allow_v1: proxy_config.allow_v1,
                allow_v2: proxy_config.allow_v2,
            };
            match ProxiedStream::create_from_tokio(stream, proxy).await {
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
            metrics::requests::reject();
            metrics::connection_duration::record(connection_start);

            if let Err(e) = stream.shutdown().await {
                debug!(cause = e.to_string(), "failed to close a client connection");
            }
            return;
        }

        let adapters = self.adapters.clone();
        let connection_config = self.config.clone();
        let shutdown = stop.child_token();
        metrics::requests::accept();

        // Create a new shutdown timeout.
        let connection_timeout = Duration::from_secs(self.config.connection_timeout);
        let _shutdown = shutdown.clone();
        self.tracker.spawn(async move {
            select! {
                _ = tokio::time::sleep(connection_timeout) => {
                    _shutdown.cancel();
                    debug!("connection timeout");
                },
                _ = _shutdown.cancelled() => {
                    debug!("connection cancelled");
                },
            }
        });

        // Create a new connection and run protocol
        self.tracker.spawn(async move {
            metrics::open_connections::inc();

            // Create the connection and wait for its completion.
            let mut connection = Connection::new(
                &mut stream,
                adapters,
                connection_config,
                client_addr,
                shutdown,
            );
            match connection.listen().await {
                Ok(()) | Err(Error::ConnectionClosed) => {
                    debug!("connection completed");
                }
                Err(err) => {
                    warn!(cause = err.to_string(), "failed to handle connection");
                }
            }

            // flush connection and shutdown
            if let Err(err) = stream.shutdown().await {
                warn!(cause = err.to_string(), "failed to shutdown connection");
            }
            info!("closed connection");

            // update metrics
            metrics::connection_duration::record(connection_start);
            metrics::open_connections::dec();
        });
    }
}
