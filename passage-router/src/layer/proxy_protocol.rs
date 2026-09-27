use crate::metrics;
use passage_core::router::Layer;
use proxy_header::ParseConfig;
use proxy_header::io::ProxiedStream;
use std::net::SocketAddr;
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::debug;

/// Reads the PROXY header a trusted load balancer puts in front of the stream, so that every layer
/// behind it sees the address of the real peer rather than that of the balancer.
pub struct ProxyProtocol {
    allow_v1: bool,
    allow_v2: bool,
}

impl ProxyProtocol {
    /// Accepts the header versions named. Refusing one of them is how a deployment pins itself to
    /// what its balancer actually sends.
    pub fn new(allow_v1: bool, allow_v2: bool) -> Self {
        Self { allow_v1, allow_v2 }
    }

    fn parse_config(&self) -> ParseConfig {
        ParseConfig {
            include_tlvs: false,
            allow_v1: self.allow_v1,
            allow_v2: self.allow_v2,
        }
    }

    async fn parse<Io>(&self, io: Io, addr: SocketAddr) -> Option<(ProxiedStream<Io>, SocketAddr)>
    where
        Io: AsyncRead + AsyncWrite + Send + Unpin + 'static,
    {
        match ProxiedStream::create_from_tokio(io, self.parse_config()).await {
            Ok(stream) => {
                let client_addr = stream
                    .proxy_header()
                    .proxied_address()
                    .map(|address| address.source)
                    .unwrap_or(addr);
                Some((stream, client_addr))
            }
            Err(err) => {
                debug!(
                    cause = %err,
                    ?addr,
                    "failed to parse proxy protocol header, connection closed",
                );
                metrics::requests::reject();
                None
            }
        }
    }
}

/// Stacks the PROXY header reader in front of the connection.
pub struct ProxyProtocolLayer {
    proxy_protocol: Option<ProxyProtocol>,
}

impl ProxyProtocolLayer {
    /// Builds the layer described by `config`. Without one no header is expected, and the address
    /// the listener reported is the one everything behind it sees.
    pub fn new(config: Option<crate::config::ProxyProtocol>) -> Self {
        Self {
            proxy_protocol: config
                .map(|config| ProxyProtocol::new(config.allow_v1, config.allow_v2)),
        }
    }
}

impl<Io> Layer<Io, SocketAddr> for ProxyProtocolLayer
where
    Io: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Io = ProxiedStream<Io>;

    async fn admit(&self, io: Io, addr: SocketAddr) -> Option<(Self::Io, SocketAddr)> {
        let Some(proxy_protocol) = self.proxy_protocol.as_ref() else {
            return Some((ProxiedStream::unproxied(io), addr));
        };
        proxy_protocol.parse(io, addr).await
    }
}
