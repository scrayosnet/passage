//! PROXY protocol support for extracting real client addresses from load balancers.
//!
//! This module provides support for the PROXY protocol (v1 and v2) which allows
//! load balancers and proxies to preserve the original client IP address.

use std::io::ErrorKind;
use std::net::SocketAddr;
use tokio::io::{AsyncRead, AsyncReadExt};
use tracing::debug;

/// Error type for PROXY protocol parsing.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("io error while reading PROXY protocol header: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid PROXY protocol header: {0}")]
    InvalidHeader(String),

    #[error("unsupported PROXY protocol version")]
    UnsupportedVersion,
}

/// Read and parse a PROXY protocol header from the stream.
///
/// This function attempts to read a PROXY protocol header (v1 or v2) from the stream.
/// If successful, it returns the real client address. If the header is invalid or
/// missing, it returns an error.
///
/// # Arguments
///
/// * `stream` - The stream to read from
///
/// # Returns
///
/// The real client address extracted from the PROXY protocol header.
pub async fn read_proxy_header<S>(stream: &mut S) -> Result<SocketAddr, Error>
where
    S: AsyncRead + Unpin,
{
    // Peek at the first byte to determine the protocol version
    let mut first_byte = [0u8; 1];
    stream.read_exact(&mut first_byte).await?;

    match first_byte[0] {
        // PROXY protocol v1 starts with 'P' (0x50)
        b'P' => read_v1_header(stream, first_byte[0]).await,
        // PROXY protocol v2 starts with 0x0D
        0x0D => read_v2_header(stream).await,
        _ => Err(Error::InvalidHeader(
            "does not start with PROXY protocol signature".to_string(),
        )),
    }
}

/// Read and parse a PROXY protocol v1 header.
///
/// Format: "PROXY TCP4/TCP6 source_ip dest_ip source_port dest_port\r\n"
async fn read_v1_header<S>(stream: &mut S, first_byte: u8) -> Result<SocketAddr, Error>
where
    S: AsyncRead + Unpin,
{
    // Read the rest of the header (up to \r\n, max 107 bytes for v1)
    let mut header = vec![first_byte];
    let mut byte = [0u8; 1];
    let mut prev_byte = first_byte;

    // Read until we find \r\n
    for _ in 0..107 {
        stream.read_exact(&mut byte).await?;
        header.push(byte[0]);

        if prev_byte == b'\r' && byte[0] == b'\n' {
            break;
        }
        prev_byte = byte[0];
    }

    // Parse the header
    let header_str = String::from_utf8_lossy(&header);
    debug!(header = %header_str, "parsing PROXY protocol v1 header");

    let parts: Vec<&str> = header_str.trim().split_whitespace().collect();

    if parts.len() < 6 {
        return Err(Error::InvalidHeader(format!(
            "v1 header has insufficient parts: {}",
            parts.len()
        )));
    }

    if parts[0] != "PROXY" {
        return Err(Error::InvalidHeader(format!(
            "v1 header does not start with PROXY: {}",
            parts[0]
        )));
    }

    // parts[1] is TCP4 or TCP6
    // parts[2] is source IP
    // parts[3] is dest IP
    // parts[4] is source port
    // parts[5] is dest port

    let source_ip = parts[2];
    let source_port = parts[4];

    // IPv6 addresses need to be wrapped in brackets when combined with port
    let addr_str = if parts[1] == "TCP6" {
        format!("[{}]:{}", source_ip, source_port)
    } else {
        format!("{}:{}", source_ip, source_port)
    };

    let addr = addr_str.parse::<SocketAddr>().map_err(|e| {
        Error::InvalidHeader(format!("failed to parse source address '{}': {}", addr_str, e))
    })?;

    debug!(addr = %addr, "extracted real client address from PROXY protocol v1");
    Ok(addr)
}

/// Read and parse a PROXY protocol v2 header.
///
/// Format: Binary format with signature, version, command, family, addresses, etc.
async fn read_v2_header<S>(stream: &mut S) -> Result<SocketAddr, Error>
where
    S: AsyncRead + Unpin,
{
    // Read the signature (remaining 11 bytes after 0x0D)
    let mut sig = [0u8; 11];
    stream.read_exact(&mut sig).await?;

    // v2 signature: 0D 0A 0D 0A 00 0D 0A 51 55 49 54 0A
    const V2_SIG: [u8; 11] = [0x0A, 0x0D, 0x0A, 0x00, 0x0D, 0x0A, 0x51, 0x55, 0x49, 0x54, 0x0A];

    if sig != V2_SIG {
        return Err(Error::InvalidHeader(
            "v2 signature mismatch".to_string(),
        ));
    }

    // Read version and command byte
    let mut ver_cmd = [0u8; 1];
    stream.read_exact(&mut ver_cmd).await?;

    let version = (ver_cmd[0] & 0xF0) >> 4;
    let command = ver_cmd[0] & 0x0F;

    if version != 2 {
        return Err(Error::UnsupportedVersion);
    }

    // Command: 0x0 = LOCAL, 0x1 = PROXY
    if command == 0x0 {
        // LOCAL command means health check, no real address
        return Err(Error::InvalidHeader(
            "LOCAL command does not provide real address".to_string(),
        ));
    }

    // Read family and protocol byte
    let mut fam_proto = [0u8; 1];
    stream.read_exact(&mut fam_proto).await?;

    let family = (fam_proto[0] & 0xF0) >> 4;
    let protocol = fam_proto[0] & 0x0F;

    // Read address length (2 bytes, big-endian)
    let mut addr_len_bytes = [0u8; 2];
    stream.read_exact(&mut addr_len_bytes).await?;
    let addr_len = u16::from_be_bytes(addr_len_bytes);

    // Read the address data
    let mut addr_data = vec![0u8; addr_len as usize];
    stream.read_exact(&mut addr_data).await?;

    debug!(
        family = family,
        protocol = protocol,
        addr_len = addr_len,
        "parsing PROXY protocol v2 header"
    );

    // Parse based on family
    match (family, protocol) {
        // IPv4 (AF_INET = 1, STREAM = 1)
        (0x1, 0x1) => {
            if addr_data.len() < 12 {
                return Err(Error::InvalidHeader(
                    "v2 IPv4 address data too short".to_string(),
                ));
            }
            let src_ip = std::net::Ipv4Addr::new(
                addr_data[0],
                addr_data[1],
                addr_data[2],
                addr_data[3],
            );
            let src_port = u16::from_be_bytes([addr_data[8], addr_data[9]]);
            let addr = SocketAddr::new(src_ip.into(), src_port);
            debug!(addr = %addr, "extracted real client address from PROXY protocol v2 (IPv4)");
            Ok(addr)
        }
        // IPv6 (AF_INET6 = 2, STREAM = 1)
        (0x2, 0x1) => {
            if addr_data.len() < 36 {
                return Err(Error::InvalidHeader(
                    "v2 IPv6 address data too short".to_string(),
                ));
            }
            let src_ip = std::net::Ipv6Addr::new(
                u16::from_be_bytes([addr_data[0], addr_data[1]]),
                u16::from_be_bytes([addr_data[2], addr_data[3]]),
                u16::from_be_bytes([addr_data[4], addr_data[5]]),
                u16::from_be_bytes([addr_data[6], addr_data[7]]),
                u16::from_be_bytes([addr_data[8], addr_data[9]]),
                u16::from_be_bytes([addr_data[10], addr_data[11]]),
                u16::from_be_bytes([addr_data[12], addr_data[13]]),
                u16::from_be_bytes([addr_data[14], addr_data[15]]),
            );
            let src_port = u16::from_be_bytes([addr_data[32], addr_data[33]]);
            let addr = SocketAddr::new(src_ip.into(), src_port);
            debug!(addr = %addr, "extracted real client address from PROXY protocol v2 (IPv6)");
            Ok(addr)
        }
        // UNSPEC (0x0) - connection without address info
        (0x0, _) => Err(Error::InvalidHeader(
            "UNSPEC family does not provide address".to_string(),
        )),
        _ => Err(Error::InvalidHeader(format!(
            "unsupported family/protocol combination: {}/{}",
            family, protocol
        ))),
    }
}
