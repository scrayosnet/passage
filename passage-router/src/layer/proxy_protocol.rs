use std::net::SocketAddr;
use tokio::io::{AsyncRead, AsyncWrite};
use passage_core::router::Layer;
use proxy_header::io::ProxiedStream;
use proxy_header::ParseConfig;
use tracing::info;

pub struct ProxyProtocol {
    allow_v1: bool,
    allow_v2: bool,
}

impl ProxyProtocol {
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
    where Io: AsyncRead + AsyncWrite + Send + Unpin + 'static,
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
            Err(e) => {
                info!(
                    cause = e.to_string(),
                    addr = addr.to_string(),
                    "failed to parse proxy protocol header, connection closed"
                );
                None
            }
        }
    }
}

pub struct ProxyProtocolLayer {
    proxy_protocol: Option<ProxyProtocol>,
}

impl ProxyProtocolLayer {
    pub fn new(config: Option<crate::config::ProxyProtocol>) -> Self {
        Self { proxy_protocol: config.map(|config| ProxyProtocol::new(config.allow_v1, config.allow_v2)) }
    }
}

impl<Io> Layer<Io, SocketAddr> for ProxyProtocolLayer
where
    Io: AsyncRead + AsyncWrite + Send + Unpin + 'static,
{
    type Io = ProxiedStream<Io>;

    async fn admit(&self, io: Io, addr: SocketAddr) -> Option<(Self::Io, SocketAddr)> {
        let Some(proxy_protocol) = self.proxy_protocol.as_ref() else {
            return Some((ProxiedStream::unproxied(io), addr))
        };
        proxy_protocol.parse(io, addr).await
    }
}
