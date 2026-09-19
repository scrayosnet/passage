use crate::common::VarInt;
use crate::wire::{Reader, WireError, Writer};
use crate::{Packet, Phase, ProtocolVersion};
use serde::Serialize;

/// The [`ServerStatusResponsePacket`].
///
/// This packet can be received only after a [`StatusRequestPacket`](super::serverbound::StatusRequestPacket) and will not close the connection, allowing for a
/// ping sequence to be exchanged afterward.
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Status_Response)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerStatusResponsePacket {
    /// The JSON response body that contains all self-reported server metadata.
    pub body: String,
}

impl ServerStatusResponsePacket {
    /// Creates a new [`ServerStatusResponsePacket`] from a serializable status. The status has to
    /// conform to the packet body.
    pub fn try_from<T: Serialize>(status: &T) -> Result<Self, serde_json::Error> {
        Ok(Self {
            body: serde_json::to_string(status)?,
        })
    }
}

impl Packet for ServerStatusResponsePacket {
    const NAME: &'static str = "server::status_response";
    const PHASE: Phase = Phase::Status;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(ProtocolVersion::UNKNOWN, 0x00)];

    fn decode(r: &mut Reader<'_>, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            body: r.string("body", 32767)?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.string("body", self.body.as_str())?;
        Ok(())
    }
}

/// This is the response to a specific [`PingPacket`](super::serverbound::PingPacket) that can be used to measure the server ping.
///
/// This packet will be sent after a corresponding [`PingPacket`](super::serverbound::PingPacket) and will have the same payload as the request. This
/// also consumes the connection, ending the Server List Ping sequence.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerPongPacket {
    /// The arbitrary payload that was sent from the client (to identify the corresponding response).
    pub payload: u64,
}

impl Packet for ServerPongPacket {
    const NAME: &'static str = "server::pong";
    const PHASE: Phase = Phase::Status;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(ProtocolVersion::UNKNOWN, 0x01)];

    fn decode(r: &mut Reader<'_>, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            payload: r.u64("payload")?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.u64(self.payload);
        Ok(())
    }
}

/// The [`ClientStatusRequestPacket`].
///
/// The status can only be requested once immediately after the handshake, before any ping. The
/// server won't respond otherwise.
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Status_Request)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClientStatusRequestPacket;

impl Packet for ClientStatusRequestPacket {
    const NAME: &'static str = "client::status_request";
    const PHASE: Phase = Phase::Status;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(ProtocolVersion::UNKNOWN, 0x00)];

    fn decode(_: &mut Reader<'_>, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self)
    }

    fn encode(&self, _: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        Ok(())
    }
}

/// The [`ClientPingPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Ping_Request_(status))
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClientPingPacket {
    /// The arbitrary payload that will be returned from the server (to identify the corresponding request).
    pub payload: u64,
}

impl Packet for ClientPingPacket {
    const NAME: &'static str = "client::ping";
    const PHASE: Phase = Phase::Status;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(ProtocolVersion::UNKNOWN, 0x01)];

    fn decode(r: &mut Reader<'_>, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            payload: r.u64("payload")?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.u64(self.payload);
        Ok(())
    }
}
