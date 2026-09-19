//! A small protocol in the shape of the real one, for the tests to speak.
//!
//! It is deliberately not Minecraft: it is the smallest set of packets that still has everything
//! the driver has to get right -- a handshake that pins a version, IDs that move between versions, a
//! field that only exists above a threshold, a packet that does not exist below one, and a phase
//! where both directions claim the same ID.

use passage_core::wire::{Reader, WireResult, Writer};
use passage_core::{Packet, Phase, ProtocolVersion, versions};
use uuid::Uuid;

/// What the peer connected for, chosen by the handshake.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// A server list ping.
    Status,
    /// A login.
    Login,
}

/// The handshake: the only packet that exists before a version is known, which is why it is
/// anchored at the floor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Handshake {
    pub version: ProtocolVersion,
    pub host: String,
    pub port: u16,
    pub intent: Intent,
}

impl Handshake {
    /// A handshake to `mc.justchunks.net`, which is what the tests call the address.
    pub fn new(version: ProtocolVersion, intent: Intent) -> Self {
        Self::to("mc.justchunks.net", version, intent)
    }

    /// A handshake to a hostname of the test's choosing.
    pub fn to(host: &str, version: ProtocolVersion, intent: Intent) -> Self {
        Self {
            version,
            host: host.to_owned(),
            port: 25_565,
            intent,
        }
    }
}

impl Packet for Handshake {
    const NAME: &'static str = "Handshake";
    const PHASE: Phase = Phase::Handshake;
    const IDS: &'static [(ProtocolVersion, i32)] = &[(ProtocolVersion::UNKNOWN, 0x00)];

    fn decode(r: &mut Reader<'_>, _version: ProtocolVersion) -> WireResult<Self> {
        Ok(Self {
            version: ProtocolVersion::new(r.var_int("protocol_version")?),
            host: r.string("server_address", 255)?,
            port: r.u16("server_port")?,
            intent: match r.var_int("intent")? {
                1 => Intent::Status,
                _ => Intent::Login,
            },
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _version: ProtocolVersion) -> WireResult<()> {
        w.var_int(self.version.get());
        w.string("server_address", &self.host)?;
        w.u16(self.port);
        w.var_int(match self.intent {
            Intent::Status => 1,
            Intent::Login => 2,
        });
        Ok(())
    }
}

/// Asks for the status. Anchored at the floor, so a client at any version can be answered -- which
/// is how it learns which version to install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusRequest;

impl Packet for StatusRequest {
    const NAME: &'static str = "StatusRequest";
    const PHASE: Phase = Phase::Status;
    const IDS: &'static [(ProtocolVersion, i32)] = &[(ProtocolVersion::UNKNOWN, 0x00)];

    fn decode(_r: &mut Reader<'_>, _version: ProtocolVersion) -> WireResult<Self> {
        Ok(Self)
    }

    fn encode(&self, _w: &mut Writer<'_>, _version: ProtocolVersion) -> WireResult<()> {
        Ok(())
    }
}

/// The answer to a [`StatusRequest`]. Claims the same ID in the same phase, in the other direction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusResponse {
    pub body: String,
}

impl StatusResponse {
    pub fn text(body: &str) -> Self {
        Self {
            body: body.to_owned(),
        }
    }
}

impl Packet for StatusResponse {
    const NAME: &'static str = "StatusResponse";
    const PHASE: Phase = Phase::Status;
    const IDS: &'static [(ProtocolVersion, i32)] = &[(ProtocolVersion::UNKNOWN, 0x00)];

    fn decode(r: &mut Reader<'_>, _version: ProtocolVersion) -> WireResult<Self> {
        Ok(Self {
            body: r.string("body", 32_000)?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _version: ProtocolVersion) -> WireResult<()> {
        w.string("body", &self.body)
    }
}

/// The ping half of the status flow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ping {
    pub payload: i64,
}

impl Packet for Ping {
    const NAME: &'static str = "Ping";
    const PHASE: Phase = Phase::Status;
    const IDS: &'static [(ProtocolVersion, i32)] = &[(ProtocolVersion::UNKNOWN, 0x01)];

    fn decode(r: &mut Reader<'_>, _version: ProtocolVersion) -> WireResult<Self> {
        Ok(Self {
            payload: r.i64("payload")?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _version: ProtocolVersion) -> WireResult<()> {
        w.i64(self.payload);
        Ok(())
    }
}

/// The pong half of the status flow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pong {
    pub payload: i64,
}

impl Packet for Pong {
    const NAME: &'static str = "Pong";
    const PHASE: Phase = Phase::Status;
    const IDS: &'static [(ProtocolVersion, i32)] = &[(ProtocolVersion::UNKNOWN, 0x01)];

    fn decode(r: &mut Reader<'_>, _version: ProtocolVersion) -> WireResult<Self> {
        Ok(Self {
            payload: r.i64("payload")?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _version: ProtocolVersion) -> WireResult<()> {
        w.i64(self.payload);
        Ok(())
    }
}

/// Starts a login.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginStart {
    pub user_name: String,
    pub user_id: Uuid,
}

impl LoginStart {
    pub fn named(user_name: &str) -> Self {
        Self {
            user_name: user_name.to_owned(),
            user_id: Uuid::nil(),
        }
    }
}

impl Packet for LoginStart {
    const NAME: &'static str = "LoginStart";
    const PHASE: Phase = Phase::Login;
    const IDS: &'static [(ProtocolVersion, i32)] = &[(ProtocolVersion::UNKNOWN, 0x00)];

    fn decode(r: &mut Reader<'_>, _version: ProtocolVersion) -> WireResult<Self> {
        Ok(Self {
            user_name: r.string("user_name", 16)?,
            user_id: r.uuid("user_id")?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _version: ProtocolVersion) -> WireResult<()> {
        w.string("user_name", &self.user_name)?;
        w.uuid(&self.user_id);
        Ok(())
    }
}

/// The answer to a [`LoginStart`], with a field that only exists from 26.1 on and an ID that moved
/// at the same threshold. An old client must not be sent either.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginSuccess {
    pub user_name: String,
    pub session_id: Option<Uuid>,
}

impl Packet for LoginSuccess {
    const NAME: &'static str = "LoginSuccess";
    const PHASE: Phase = Phase::Login;
    const IDS: &'static [(ProtocolVersion, i32)] =
        &[(versions::V26_1, 0x02), (ProtocolVersion::UNKNOWN, 0x01)];

    fn decode(r: &mut Reader<'_>, version: ProtocolVersion) -> WireResult<Self> {
        Ok(Self {
            user_name: r.string("user_name", 16)?,
            session_id: r.gated(version.at_least(versions::V26_1), |r| r.uuid("session_id"))?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> WireResult<()> {
        w.string("user_name", &self.user_name)?;
        if version.at_least(versions::V26_1) {
            w.uuid(&self.session_id.unwrap_or_else(Uuid::nil));
        }
        Ok(())
    }
}

/// Acknowledges a [`LoginSuccess`]. A client that sends it before it could have seen one is
/// breaking the protocol, which is what the read gate exists to notice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginAcknowledged;

impl Packet for LoginAcknowledged {
    const NAME: &'static str = "LoginAcknowledged";
    const PHASE: Phase = Phase::Login;
    const IDS: &'static [(ProtocolVersion, i32)] = &[(ProtocolVersion::UNKNOWN, 0x03)];

    fn decode(_r: &mut Reader<'_>, _version: ProtocolVersion) -> WireResult<Self> {
        Ok(Self)
    }

    fn encode(&self, _w: &mut Writer<'_>, _version: ProtocolVersion) -> WireResult<()> {
        Ok(())
    }
}

/// Why the peer is being turned away. Anchored at the floor, because the client it is most often
/// sent to is one whose version resolves nothing else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disconnect {
    pub reason: String,
}

impl Disconnect {
    pub fn text(reason: &str) -> Self {
        Self {
            reason: reason.to_owned(),
        }
    }
}

impl Packet for Disconnect {
    const NAME: &'static str = "Disconnect";
    const PHASE: Phase = Phase::Login;
    const IDS: &'static [(ProtocolVersion, i32)] = &[(ProtocolVersion::UNKNOWN, 0x00)];

    fn decode(r: &mut Reader<'_>, _version: ProtocolVersion) -> WireResult<Self> {
        Ok(Self {
            reason: r.string("reason", 256)?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _version: ProtocolVersion) -> WireResult<()> {
        w.string("reason", &self.reason)
    }
}

/// Sends the peer somewhere else. It does not exist before 1.20.5, so encoding it for a client
/// below that threshold is a mistake the codec can catch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfer {
    pub host: String,
    pub port: u16,
}

impl Packet for Transfer {
    const NAME: &'static str = "Transfer";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, i32)] = &[(versions::V1_20_5, 0x0B)];

    fn decode(r: &mut Reader<'_>, _version: ProtocolVersion) -> WireResult<Self> {
        Ok(Self {
            host: r.string("host", 255)?,
            port: r.u16("port")?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _version: ProtocolVersion) -> WireResult<()> {
        w.string("host", &self.host)?;
        w.u16(self.port);
        Ok(())
    }
}

/// Keeps a connection alive while something slow runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeepAlive {
    pub id: i64,
}

impl Packet for KeepAlive {
    const NAME: &'static str = "KeepAlive";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, i32)] = &[(ProtocolVersion::UNKNOWN, 0x04)];

    fn decode(r: &mut Reader<'_>, _version: ProtocolVersion) -> WireResult<Self> {
        Ok(Self { id: r.i64("id")? })
    }

    fn encode(&self, w: &mut Writer<'_>, _version: ProtocolVersion) -> WireResult<()> {
        w.i64(self.id);
        Ok(())
    }
}

/// The answer to a [`KeepAlive`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeepAliveResponse {
    pub id: i64,
}

impl Packet for KeepAliveResponse {
    const NAME: &'static str = "KeepAliveResponse";
    const PHASE: Phase = Phase::Configuration;
    const IDS: &'static [(ProtocolVersion, i32)] = &[(ProtocolVersion::UNKNOWN, 0x04)];

    fn decode(r: &mut Reader<'_>, _version: ProtocolVersion) -> WireResult<Self> {
        Ok(Self { id: r.i64("id")? })
    }

    fn encode(&self, w: &mut Writer<'_>, _version: ProtocolVersion) -> WireResult<()> {
        w.i64(self.id);
        Ok(())
    }
}
