#[cfg(any(feature = "server", feature = "client"))]
use crate::Error;
use crate::{Packet, State, VarInt};

pub mod serverbound {
    use super::{Error, Packet, State, VarInt};
    #[cfg(feature = "client")]
    use crate::io::reader::{Read, ReadBytesExt, ReadPacket, ReadPacketExt};
    #[cfg(feature = "server")]
    use crate::io::writer::{Write, WriteBytesExt, WritePacket, WritePacketExt};
    use byteorder::BigEndian;
    #[cfg(test)]
    use fake::Dummy;
    use tracing::instrument;

    /// The [`HandshakePacket`].
    ///
    /// This packet causes the server to switch into the target state. It should be sent right after
    /// opening the TCP connection to prevent the server from disconnecting.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Handshake)
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct HandshakePacket {
        /// The pretended protocol version.
        pub protocol_version: VarInt,
        /// The pretended server address.
        pub server_address: String,
        /// The pretended server port.
        pub server_port: u16,
        /// The protocol states to initiate.
        pub next_state: State,
    }

    impl Packet for HandshakePacket {
        const ID: VarInt = 0x00;
    }

    #[cfg(feature = "client")]
    impl WritePacket for HandshakePacket {
        fn write_packet(&self, dst: &mut impl Write) -> Result<(), Error> {
            dst.write_varint(self.protocol_version)?;
            dst.write_string(&self.server_address)?;
            dst.write_u16::<BigEndian>(self.server_port)?;
            dst.write_varint(self.next_state.into())?;

            Ok(())
        }
    }

    #[cfg(feature = "server")]
    impl ReadPacket for HandshakePacket {
        #[instrument(skip_all, fields(packet_type = std::any::type_name::<Self>()))]
        fn read_packet(src: &mut impl Read) -> Result<Self, Error> {
            let protocol_version = src.read_varint()?;
            let server_address = src.read_string()?;
            let server_port = src.read_u16::<BigEndian>()?;
            let next_state = src.read_varint()?.try_into()?;

            Ok(Self {
                protocol_version,
                server_address,
                server_port,
                next_state,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::tests::assert_packet;

    #[test]
    fn write_read_serverbound_handshake_packet() {
        assert_packet::<serverbound::HandshakePacket>(0x00);
    }
}
