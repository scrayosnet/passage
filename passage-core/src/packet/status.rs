use crate::common::VarInt;
use crate::wire::{Reader, WireError, Writer};
use crate::{Packet, Phase, ProtocolVersion};
use serde::Serialize;

/// The [`ServerStatusResponsePacket`].
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

/// The [`ServerPongResponsePacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Pong_Response_(status))
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerPongResponsePacket {
    /// The arbitrary payload that was sent from the client (to identify the corresponding response).
    pub payload: u64,
}

impl Packet for ServerPongResponsePacket {
    const NAME: &'static str = "server::pong_response";
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

/// The [`ClientPingRequestPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Ping_Request_(status))
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClientPingRequestPacket {
    /// The arbitrary payload that will be returned from the server (to identify the corresponding request).
    pub payload: u64,
}

impl Packet for ClientPingRequestPacket {
    const NAME: &'static str = "client::ping_request";
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
