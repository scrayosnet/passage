use crate::error::Error;
use crate::metrics;
use crate::protocol::config::Config;
use crate::protocol::connection::Connection;
use crate::rate_limiter::RateLimiter;
use passage_adapters::Adapters;
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

// the server protocol
pub struct Listener<Stat, Disc, Filt, Stra, Auth, Loca> {
    adapters: Arc<Adapters<Stat, Disc, Filt, Stra, Auth, Loca>>,
    tracker: TaskTracker,
    rate_limiter: Option<RateLimiter<IpAddr>>,
    config: Config,
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
        adapters: Arc<Adapters<Stat, Disc, Filt, Stra, Auth, Loca>>,
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
            "starting protocol"
        );
        let listener = TcpListener::bind(address).await?;
        loop {
            // accept the next incoming connection
            let (stream, addr) = select! {
                accepted = listener.accept() => accepted?,
                _ = stop.cancelled() => {
                    info!("stopping protocol");
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

    async fn handle_proxy(
        &mut self,
        stream: TcpStream,
        addr: SocketAddr,
    ) -> Result<(ProxiedStream<TcpStream>, SocketAddr), std::io::Error> {
        let Some(proxy_config) = &self.config.proxy_protocol else {
            return Ok((ProxiedStream::unproxied(stream), addr));
        };

        let proxy = ParseConfig {
            include_tlvs: false,
            allow_v1: proxy_config.allow_v1,
            allow_v2: proxy_config.allow_v2,
        };

        let stream = ProxiedStream::create_from_tokio(stream, proxy).await?;
        let client_addr = stream
            .proxy_header()
            .proxied_address()
            .map(|address| address.source)
            .unwrap_or(addr);
        Ok((stream, client_addr))
    }

    #[instrument(skip(self, stream))]
    async fn handle(&mut self, stream: TcpStream, addr: SocketAddr) {
        let connection_start = Instant::now();

        // wrap stream with an optional proxy protocol parser
        let (mut stream, client_addr) = match self.handle_proxy(stream, addr).await {
            Ok((stream, client_addr)) => {
                debug!(addr = %client_addr, "handling new connection");
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
        };

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

        let adapters = self.adapters.clone();
        let connection_config = self.config.clone();
        let connection_timeout = Duration::from_secs(self.config.connection_timeout);

        // create a new connection and run protocol
        self.tracker.spawn(async move {
            metrics::open_connections::inc();
            let mut connection =
                Connection::new(&mut stream, adapters, connection_config, client_addr);

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
