//! A worked packet set: enough of the protocol to serve a status ping and a login.
//!
//! The point of this module is the *declarations*, not the coverage. Two kinds of version drift
//! appear here, and neither needs a second packet type:
//!
//! * [`LoginSuccess`] gained a trailing session ID in the 26.2 protocol -- expressed as
//!   `= since(LoginSuccessSessionId)`.
//! * [`Transfer`] does not exist before 1.20.5 -- expressed by its `ids` table starting there, so
//!   sending it to an older client is an internal error instead of a malformed frame.

use crate::error::Result;
use crate::packet;
use crate::version::{ProtocolVersion, versions};
use crate::wire::{Reader, VarInt, Wire, Writer};
use uuid::Uuid;

/// A signed profile property, as carried by `LoginSuccess`.
///
/// Hand-written rather than generated: composite wire types that are not packets implement
/// [`Wire`] directly, and the generated packet codecs pick them up like any primitive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Property {
    /// The property name, e.g. `textures`.
    pub name: String,
    /// The property value.
    pub value: String,
}

impl Wire for Property {
    fn read(
        reader: &mut Reader<'_>,
        version: ProtocolVersion,
        field: &'static str,
    ) -> Result<Self> {
        Ok(Self {
            name: String::read(reader, version, field)?,
            value: String::read(reader, version, field)?,
        })
    }

    fn write(&self, writer: &mut Writer<'_>, version: ProtocolVersion) {
        self.name.write(writer, version);
        self.value.write(writer, version);
    }
}

packet! {
    /// The handshake, the first packet on every connection.
    ///
    /// Its ID table starts at [`ProtocolVersion::UNKNOWN`] because it has to be decodable *before*
    /// a version is known -- which is exactly what the version-keyed table expresses naturally.
    pub struct Intention {
        /// The protocol version the client speaks.
        pub protocol_version: VarInt,
        /// The hostname the client connected to, used for routing.
        pub server_address: String,
        /// The port the client connected to.
        pub server_port: u16,
        /// 1 = status, 2 = login, 3 = transfer.
        pub intent: VarInt,
    }
    phase = Handshake;
    direction = Serverbound;
    ids = [ ProtocolVersion::UNKNOWN => 0x00 ];
}

packet! {
    /// The client asking for the server list entry.
    pub struct StatusRequest {}
    phase = Status;
    direction = Serverbound;
    ids = [ ProtocolVersion::UNKNOWN => 0x00 ];
}

packet! {
    /// The server list entry.
    pub struct StatusResponse {
        /// The JSON body.
        pub body: String,
    }
    phase = Status;
    direction = Clientbound;
    ids = [ ProtocolVersion::UNKNOWN => 0x00 ];
}

packet! {
    /// The latency probe.
    pub struct PingRequest {
        /// An opaque payload to echo back.
        pub payload: i64,
    }
    phase = Status;
    direction = Serverbound;
    ids = [ ProtocolVersion::UNKNOWN => 0x01 ];
}

packet! {
    /// The echo of [`PingRequest`].
    pub struct PongResponse {
        /// The echoed payload.
        pub payload: i64,
    }
    phase = Status;
    direction = Clientbound;
    ids = [ ProtocolVersion::UNKNOWN => 0x01 ];
}

packet! {
    /// The start of the login phase.
    pub struct LoginStart {
        /// The (unverified) name the client claims.
        pub user_name: String,
        /// The (unverified) profile id the client claims.
        pub user_id: Uuid,
    }
    phase = Login;
    direction = Serverbound;
    ids = [ versions::V1_20_5 => 0x00 ];
}

packet! {
    /// The end of the login phase, sent by the client.
    pub struct LoginAcknowledged {}
    phase = Login;
    direction = Serverbound;
    ids = [ versions::V1_20_5 => 0x03 ];
}

packet! {
    /// The authenticated profile.
    ///
    /// This is the packet the 26.2 protocol quick-fix hard-coded: it grew a trailing session ID,
    /// and writing it unconditionally breaks every older client. As a version-gated field there is
    /// one type, one codec, and no version left behind.
    pub struct LoginSuccess {
        /// The authenticated profile id.
        pub user_id: Uuid,
        /// The authenticated profile name.
        pub user_name: String,
        /// The signed profile properties.
        pub properties: Vec<Property>,
        /// The session id. Only on the wire since the 26.2 protocol.
        pub session_id: Option<Uuid> = since(LoginSuccessSessionId),
    }
    phase = Login;
    direction = Clientbound;
    ids = [ versions::V1_20_5 => 0x02 ];
}

packet! {
    /// Tells the client to reconnect to another server.
    ///
    /// Does not exist before 1.20.5, which the ID table states outright.
    pub struct Transfer {
        /// The target host.
        pub host: String,
        /// The target port.
        pub port: VarInt,
    }
    phase = Configuration;
    direction = Clientbound;
    ids = [ versions::V1_20_5 => 0x0B ];
}

packet! {
    /// A keep-alive, sent by the server during the configuration phase.
    pub struct KeepAlive {
        /// The id to be echoed by the client.
        pub id: i64,
    }
    phase = Configuration;
    direction = Clientbound;
    ids = [ versions::V1_20_5 => 0x04 ];
}

packet! {
    /// The client's answer to a [`KeepAlive`].
    pub struct KeepAliveResponse {
        /// The echoed id.
        pub id: i64,
    }
    phase = Configuration;
    direction = Serverbound;
    ids = [ versions::V1_20_5 => 0x04 ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::Packet;
    use crate::wire::Limits;
    use bytes::BytesMut;

    fn roundtrip<P: Packet + PartialEq + std::fmt::Debug>(
        packet: &P,
        version: ProtocolVersion,
    ) -> P {
        let mut buf = BytesMut::new();
        packet
            .encode(&mut Writer::new(&mut buf), version)
            .expect("encodes");
        let mut reader = Reader::new(&buf, Limits::default());
        P::decode(&mut reader, version).expect("decodes")
    }

    fn login_success(session_id: Option<Uuid>) -> LoginSuccess {
        LoginSuccess {
            user_id: Uuid::from_u128(0x1234),
            user_name: "Hydrofin".to_owned(),
            properties: vec![Property {
                name: "textures".to_owned(),
                value: "...".to_owned(),
            }],
            session_id,
        }
    }

    #[test]
    fn one_type_serves_both_versions() {
        // Old client: the field is absent from the wire and decodes back to `None`.
        let old = login_success(None);
        assert_eq!(roundtrip(&old, versions::V1_21), old);

        // New client: the field is on the wire and survives the roundtrip.
        let new = login_success(Some(Uuid::from_u128(0x5678)));
        assert_eq!(roundtrip(&new, versions::V26_2), new);
    }

    #[test]
    fn the_gated_field_changes_the_encoding() {
        let packet = login_success(Some(Uuid::from_u128(0x5678)));
        let mut old = BytesMut::new();
        let mut new = BytesMut::new();
        packet
            .encode(&mut Writer::new(&mut old), versions::V1_21)
            .expect("encodes");
        packet
            .encode(&mut Writer::new(&mut new), versions::V26_2)
            .expect("encodes");
        // Exactly the 16 bytes of the session id.
        assert_eq!(new.len(), old.len() + 16);
    }

    #[test]
    fn a_missing_required_field_fails_closed() {
        // Sending a 26.2 client a packet without its session id would truncate the frame. The
        // encoder refuses instead of guessing a default.
        let err = login_success(None)
            .encode(&mut Writer::new(&mut BytesMut::new()), versions::V26_2)
            .expect_err("must refuse");
        assert!(matches!(
            err,
            crate::error::Error::Internal(crate::error::InternalError::MissingField {
                field: "session_id",
                ..
            })
        ));
    }

    #[test]
    fn packets_report_the_versions_they_exist_in() {
        assert_eq!(Transfer::id(versions::V1_20_5), Some(0x0B));
        assert_eq!(Transfer::id(ProtocolVersion::new(765)), None);
        // The handshake is readable before a version is known.
        assert_eq!(Intention::id(ProtocolVersion::UNKNOWN), Some(0x00));
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let mut buf = BytesMut::new();
        let mut writer = Writer::new(&mut buf);
        writer.i64(42);
        writer.u8(0xFF);
        let err = PingRequest::decode(&mut Reader::new(&buf, Limits::default()), versions::V1_21)
            .expect_err("must reject");
        assert_eq!(err.label(), "trailing_bytes");
    }
}
