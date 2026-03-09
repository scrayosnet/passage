#[cfg(any(feature = "server", feature = "client"))]
use crate::Error;
use crate::Packet;
use crate::VarInt;

pub mod clientbound {
    use super::{Error, Packet, VarInt};
    #[cfg(feature = "client")]
    use crate::io::reader::{Read, ReadBytesExt, ReadPacket, ReadPacketExt};
    #[cfg(feature = "server")]
    use crate::io::writer::{Write, WriteBytesExt, WritePacket, WritePacketExt};
    use byteorder::BigEndian;
    #[cfg(test)]
    use fake::Dummy;
    use tracing::instrument;

    /// The [`StatusResponsePacket`].
    ///
    /// This packet can be received only after a [`StatusRequestPacket`] and will not close the connection, allowing for a
    /// ping sequence to be exchanged afterward.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Status_Response)
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct StatusResponsePacket {
        /// The JSON response body that contains all self-reported server metadata.
        pub body: String,
    }

    impl Packet for StatusResponsePacket {
        const ID: VarInt = 0x00;
    }

    #[cfg(feature = "server")]
    impl WritePacket for StatusResponsePacket {
        fn write_packet(&self, dst: &mut impl Write) -> Result<(), Error> {
            dst.write_string(&self.body)?;

            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for StatusResponsePacket {
        #[instrument(skip_all, fields(packet_type = std::any::type_name::<Self>()))]
        fn read_packet(src: &mut impl Read) -> Result<Self, Error> {
            let body = src.read_string()?;

            Ok(Self { body })
        }
    }

    /// This is the response to a specific [`PingPacket`] that can be used to measure the server ping.
    ///
    /// This packet will be sent after a corresponding [`PingPacket`] and will have the same payload as the request. This
    /// also consumes the connection, ending the Server List Ping sequence.
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct PongPacket {
        /// The arbitrary payload that was sent from the client (to identify the corresponding response).
        pub payload: u64,
    }

    impl Packet for PongPacket {
        const ID: VarInt = 0x01;
    }

    #[cfg(feature = "server")]
    impl WritePacket for PongPacket {
        fn write_packet(&self, dst: &mut impl Write) -> Result<(), Error> {
            dst.write_u64::<BigEndian>(self.payload)?;

            Ok(())
        }
    }

    #[cfg(feature = "client")]
    impl ReadPacket for PongPacket {
        #[instrument(skip_all, fields(packet_type = std::any::type_name::<Self>()))]
        fn read_packet(src: &mut impl Read) -> Result<Self, Error> {
            let payload = src.read_u64::<BigEndian>()?;

            Ok(Self { payload })
        }
    }
}

pub mod serverbound {
    use super::{Error, Packet, VarInt};
    #[cfg(feature = "server")]
    use crate::io::reader::{Read, ReadBytesExt, ReadPacket};
    #[cfg(feature = "client")]
    use crate::io::writer::{Write, WriteBytesExt, WritePacket};
    use byteorder::BigEndian;
    #[cfg(test)]
    use fake::Dummy;
    use tracing::instrument;

    /// The [`StatusRequestPacket`].
    ///
    /// The status can only be requested once immediately after the handshake, before any ping. The
    /// server won't respond otherwise.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Status_Request)
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct StatusRequestPacket;

    impl Packet for StatusRequestPacket {
        const ID: VarInt = 0x00;
    }

    #[cfg(feature = "client")]
    impl WritePacket for StatusRequestPacket {
        fn write_packet(&self, _dst: &mut impl Write) -> Result<(), Error> {
            Ok(())
        }
    }

    #[cfg(feature = "server")]
    impl ReadPacket for StatusRequestPacket {
        #[instrument(skip_all, fields(packet_type = std::any::type_name::<Self>()))]
        fn read_packet(_src: &mut impl Read) -> Result<Self, Error> {
            Ok(Self)
        }
    }

    /// The [`PingPacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Ping_Request_(status))
    #[derive(Debug, Clone, Eq, PartialEq)]
    #[cfg_attr(test, derive(Dummy))]
    pub struct PingPacket {
        /// The arbitrary payload that will be returned from the server (to identify the corresponding request).
        pub payload: u64,
    }

    impl Packet for PingPacket {
        const ID: VarInt = 0x01;
    }

    #[cfg(feature = "client")]
    impl WritePacket for PingPacket {
        fn write_packet(&self, dst: &mut impl Write) -> Result<(), Error> {
            dst.write_u64::<BigEndian>(self.payload)?;

            Ok(())
        }
    }

    #[cfg(feature = "server")]
    impl ReadPacket for PingPacket {
        #[instrument(skip_all, fields(packet_type = std::any::type_name::<Self>()))]
        fn read_packet(src: &mut impl Read) -> Result<Self, Error> {
            let payload = src.read_u64::<BigEndian>()?;

            Ok(Self { payload })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::tests::assert_packet;

    #[test]
    fn write_read_clientbound_status_response_packet() {
        assert_packet::<clientbound::StatusResponsePacket>(0x00);
    }

    #[test]
    fn write_read_clientbound_pong_packet() {
        assert_packet::<clientbound::PongPacket>(0x01);
    }

    #[test]
    fn write_read_serverbound_status_request_packet() {
        assert_packet::<serverbound::StatusRequestPacket>(0x00);
    }
}
