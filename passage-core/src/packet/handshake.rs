use crate::common::{State, VarInt};
use crate::wire::{Reader, WireError, Writer};
use crate::{Packet, Phase, ProtocolVersion};
use bytestring::ByteString;

/// The [`ClientIntentionPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Handshake)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClientIntentionPacket {
    /// The pretended protocol version.
    pub protocol_version: ProtocolVersion,
    /// The pretended server address.
    pub server_address: ByteString,
    /// The pretended server port.
    pub server_port: u16,
    /// The protocol states to initiate.
    pub next_state: State,
}

impl Packet for ClientIntentionPacket {
    const NAME: &'static str = "client::intention";
    const PHASE: Phase = Phase::Handshake;
    const IDS: &'static [(ProtocolVersion, i32)] = &[(ProtocolVersion::UNKNOWN, 0x00)];

    fn decode(r: &mut Reader, version: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            protocol_version: r.var_int("protocol_version")?.into(),
            server_address: r.string("server_address", 255)?,
            server_port: r.u16("server_port")?,
            next_state: r.property(version, "next_state")?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> Result<(), WireError> {
        w.var_int(self.protocol_version.get());
        w.string("server_address", &self.server_address)?;
        w.u16(self.server_port);
        w.property(version, "next_state", &self.next_state)?;
        Ok(())
    }
}
