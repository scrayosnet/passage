//! A worked packet set: enough of the protocol to serve a status ping and a login.
//!
//! The point of this module is the *declarations*, not the coverage. Every codec is written out by
//! hand, which is what lets each of them say something a generated one could not:
//!
//! * [`Intention`] decodes its `intent` field into an [`Intent`], so an invalid value is a decode
//!   error and no handler can ever see one. It also caps `server_address` at 255 bytes, where a
//!   generated codec could only fall back on the frame limit.
//! * [`LoginSuccess`] gained a trailing session ID in the 26.2 protocol -- one `if` in each
//!   direction, and one type that serves every version.
//! * [`Transfer`] does not exist before 1.20.5 -- expressed by its ID table starting there, so
//!   sending it to an older client is an internal error instead of a malformed frame.

use crate::error::{InternalError, ProtocolError, Result};
use crate::packet::{Direction, Packet, Phase};
use crate::version::{ProtocolVersion, versions};
use crate::wire::{Reader, Wire, Writer};
use uuid::Uuid;

/// A hostname is at most 253 bytes; 255 leaves room for the odd trailing dot.
const MAX_HOST_LEN: usize = 255;

/// Minecraft names are 16 characters, and the protocol has always agreed.
const MAX_NAME_LEN: usize = 16;

/// A status response is JSON with a MOTD, a favicon and a sample; generous but bounded.
///
/// A field limit has to leave room for the rest of its frame, or it is not a limit at all -- the
/// frame refuses first and reports the wrong thing. These sit well under
/// [`Limits::max_frame_len`](crate::wire::Limits::max_frame_len), which is what makes them the
/// bound that actually fires.
const MAX_STATUS_LEN: usize = 16_384;

/// A signed texture blob is the largest property value in practice: roughly 2 KB of base64 and a
/// signature.
const MAX_PROPERTY_LEN: usize = 8_192;

/// Vanilla sends one property (`textures`); the bound only has to be sane.
const MAX_PROPERTIES: usize = 16;

/// What the client said it wanted in the handshake.
///
/// A domain type, not the raw `VarInt` it arrives as. That is the difference the hand-written
/// decoder buys: validation happens once, in [`Intention::decode`], instead of in every handler
/// that looks at the field.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Intent {
    /// Server list ping.
    #[default]
    Status,
    /// A fresh login.
    Login,
    /// A login continuing from another server's transfer.
    Transfer,
}

impl Intent {
    /// Decodes the wire value, rejecting anything the protocol does not define.
    pub fn from_wire(value: i32) -> Result<Self> {
        match value {
            1 => Ok(Intent::Status),
            2 => Ok(Intent::Login),
            3 => Ok(Intent::Transfer),
            _ => Err(ProtocolError::InvalidValue {
                field: "intent",
                value,
            }
            .into()),
        }
    }

    /// The wire value.
    #[must_use]
    pub fn to_wire(self) -> i32 {
        match self {
            Intent::Status => 1,
            Intent::Login => 2,
            Intent::Transfer => 3,
        }
    }
}

/// A signed profile property, as carried by [`LoginSuccess`].
///
/// A composite that appears in more than one packet, so it implements [`Wire`]. Primitives do not:
/// the call picks the encoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Property {
    /// The property name, e.g. `textures`.
    pub name: String,
    /// The property value.
    pub value: String,
}

impl Wire for Property {
    fn read(r: &mut Reader<'_>, _version: ProtocolVersion) -> Result<Self> {
        Ok(Self {
            name: r.string("property_name", MAX_NAME_LEN * 4)?,
            value: r.string("property_value", MAX_PROPERTY_LEN)?,
        })
    }

    fn write(&self, w: &mut Writer<'_>, _version: ProtocolVersion) -> Result<()> {
        w.string(&self.name)?;
        w.string(&self.value)
    }
}

/// The handshake, the first packet on every connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Intention {
    /// The protocol version the client speaks.
    pub protocol_version: ProtocolVersion,
    /// The hostname the client connected to, used for routing.
    pub server_address: String,
    /// The port the client connected to.
    pub server_port: u16,
    /// What the client wants to do.
    pub intent: Intent,
}

impl Packet for Intention {
    const NAME: &'static str = "Intention";
    const PHASE: Phase = Phase::Handshake;
    const DIRECTION: Direction = Direction::Serverbound;

    /// Version-independent, because this has to decode *before* a version is known -- which is
    /// exactly what an ID table anchored at [`ProtocolVersion::UNKNOWN`] expresses.
    const IDS: &'static [(ProtocolVersion, i32)] = &[(ProtocolVersion::UNKNOWN, 0x00)];

    fn decode(r: &mut Reader<'_>, _version: ProtocolVersion) -> Result<Self> {
        Ok(Self {
            protocol_version: ProtocolVersion::new(r.var_int()?),
            server_address: r.string("server_address", MAX_HOST_LEN)?,
            server_port: r.u16()?,
            intent: Intent::from_wire(r.var_int()?)?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _version: ProtocolVersion) -> Result<()> {
        w.var_int(self.protocol_version.get());
        w.string(&self.server_address)?;
        w.u16(self.server_port);
        w.var_int(self.intent.to_wire());
        Ok(())
    }
}

/// The client asking for the server list entry.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct StatusRequest;

impl Packet for StatusRequest {
    const NAME: &'static str = "StatusRequest";
    const PHASE: Phase = Phase::Status;
    const DIRECTION: Direction = Direction::Serverbound;

    const IDS: &'static [(ProtocolVersion, i32)] = &[(ProtocolVersion::UNKNOWN, 0x00)];

    fn decode(_r: &mut Reader<'_>, _version: ProtocolVersion) -> Result<Self> {
        Ok(Self)
    }

    fn encode(&self, _w: &mut Writer<'_>, _version: ProtocolVersion) -> Result<()> {
        Ok(())
    }
}

/// The server list entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusResponse {
    /// The JSON body.
    pub body: String,
}

impl Packet for StatusResponse {
    const NAME: &'static str = "StatusResponse";
    const PHASE: Phase = Phase::Status;
    const DIRECTION: Direction = Direction::Clientbound;

    const IDS: &'static [(ProtocolVersion, i32)] = &[(ProtocolVersion::UNKNOWN, 0x00)];

    fn decode(r: &mut Reader<'_>, _version: ProtocolVersion) -> Result<Self> {
        Ok(Self {
            body: r.string("body", MAX_STATUS_LEN)?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _version: ProtocolVersion) -> Result<()> {
        w.string(&self.body)
    }
}

/// The latency probe.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PingRequest {
    /// An opaque payload to echo back.
    pub payload: i64,
}

impl Packet for PingRequest {
    const NAME: &'static str = "PingRequest";
    const PHASE: Phase = Phase::Status;
    const DIRECTION: Direction = Direction::Serverbound;

    const IDS: &'static [(ProtocolVersion, i32)] = &[(ProtocolVersion::UNKNOWN, 0x01)];

    fn decode(r: &mut Reader<'_>, _version: ProtocolVersion) -> Result<Self> {
        Ok(Self { payload: r.i64()? })
    }

    fn encode(&self, w: &mut Writer<'_>, _version: ProtocolVersion) -> Result<()> {
        w.i64(self.payload);
        Ok(())
    }
}

/// The echo of [`PingRequest`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PongResponse {
    /// The echoed payload.
    pub payload: i64,
}

impl Packet for PongResponse {
    const NAME: &'static str = "PongResponse";
    const PHASE: Phase = Phase::Status;
    const DIRECTION: Direction = Direction::Clientbound;

    const IDS: &'static [(ProtocolVersion, i32)] = &[(ProtocolVersion::UNKNOWN, 0x01)];

    fn decode(r: &mut Reader<'_>, _version: ProtocolVersion) -> Result<Self> {
        Ok(Self { payload: r.i64()? })
    }

    fn encode(&self, w: &mut Writer<'_>, _version: ProtocolVersion) -> Result<()> {
        w.i64(self.payload);
        Ok(())
    }
}

/// The start of the login phase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginStart {
    /// The (unverified) name the client claims.
    pub user_name: String,
    /// The (unverified) profile ID the client claims.
    pub user_id: Uuid,
}

impl Packet for LoginStart {
    const NAME: &'static str = "LoginStart";
    const PHASE: Phase = Phase::Login;
    const DIRECTION: Direction = Direction::Serverbound;

    const IDS: &'static [(ProtocolVersion, i32)] = &[(versions::V1_20_5, 0x00)];

    fn decode(r: &mut Reader<'_>, _version: ProtocolVersion) -> Result<Self> {
        Ok(Self {
            user_name: r.string("user_name", MAX_NAME_LEN * 4)?,
            user_id: r.uuid()?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _version: ProtocolVersion) -> Result<()> {
        w.string(&self.user_name)?;
        w.uuid(&self.user_id);
        Ok(())
    }
}

/// The end of the login phase, sent by the client.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct LoginAcknowledged;

impl Packet for LoginAcknowledged {
    const NAME: &'static str = "LoginAcknowledged";
    const PHASE: Phase = Phase::Login;
    const DIRECTION: Direction = Direction::Serverbound;

    const IDS: &'static [(ProtocolVersion, i32)] = &[(versions::V1_20_5, 0x03)];

    fn decode(_r: &mut Reader<'_>, _version: ProtocolVersion) -> Result<Self> {
        Ok(Self)
    }

    fn encode(&self, _w: &mut Writer<'_>, _version: ProtocolVersion) -> Result<()> {
        Ok(())
    }
}

/// The authenticated profile.
///
/// This is the packet the 26.2 protocol quick-fix hard-coded: it grew a trailing session ID, and
/// writing it unconditionally breaks every older client. As a version-gated field there is one
/// type, one codec, and no version left behind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginSuccess {
    /// The authenticated profile ID.
    pub user_id: Uuid,
    /// The authenticated profile name.
    pub user_name: String,
    /// The signed profile properties.
    pub properties: Vec<Property>,
    /// The session ID. Only on the wire since the 26.2 protocol.
    pub session_id: Option<Uuid>,
}

impl Packet for LoginSuccess {
    const NAME: &'static str = "LoginSuccess";
    const PHASE: Phase = Phase::Login;
    const DIRECTION: Direction = Direction::Clientbound;

    const IDS: &'static [(ProtocolVersion, i32)] = &[(versions::V1_20_5, 0x02)];

    fn decode(r: &mut Reader<'_>, version: ProtocolVersion) -> Result<Self> {
        Ok(Self {
            user_id: r.uuid()?,
            user_name: r.string("user_name", MAX_NAME_LEN * 4)?,
            properties: r.array("properties", MAX_PROPERTIES, version)?,
            // Not a field that may be missing -- a field this version does not have.
            session_id: r.gated(version.at_least(versions::V26_2), Reader::uuid)?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> Result<()> {
        w.uuid(&self.user_id);
        w.string(&self.user_name)?;
        w.array(&self.properties, version)?;

        if version.at_least(versions::V26_2) {
            // Fail closed. Emitting a frame that is one field short would desynchronise the client
            // with nothing to diagnose it from, so this refuses rather than guessing a default.
            let session_id = self.session_id.ok_or(InternalError::MissingField {
                packet: Self::NAME,
                field: "session_id",
                version,
            })?;
            w.uuid(&session_id);
        }
        Ok(())
    }
}

/// Ends a login with a reason the client shows the player.
///
/// This is the packet that makes [`Dispatcher::on_error`](crate::conn::Dispatcher::on_error) worth
/// having: without it, a refused login and a crashed server look identical from the outside.
///
/// It is a *login-phase* packet, and its configuration-phase counterpart is a different type with a
/// different ID and a different encoding -- a JSON string here, a network-NBT text component there.
/// That is the honest reason phase is part of a packet's identity, and the reason this demo can
/// only speak in the login phase: [`wire`](crate::wire) has no NBT yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginDisconnect {
    /// The reason, as a JSON text component.
    pub reason: String,
}

impl LoginDisconnect {
    /// The reason as a plain message, which is the whole of what this demo needs.
    #[must_use]
    pub fn text(reason: &str) -> Self {
        Self {
            // Not `serde_json`: the driver has no serialisation dependency, and a demo does not
            // earn one. A real packet set would build a text component properly.
            reason: format!(r#"{{"text":"{}"}}"#, reason.replace('"', "'")),
        }
    }
}

impl Packet for LoginDisconnect {
    const NAME: &'static str = "LoginDisconnect";
    const PHASE: Phase = Phase::Login;
    const DIRECTION: Direction = Direction::Clientbound;
    /// Anchored at [`ProtocolVersion::UNKNOWN`], and that is not laziness: this ID has not moved
    /// since the login phase existed, and the packet has to reach clients too old for anything
    /// else -- telling a 1.20.4 player which version to install is the *only* thing that connection
    /// is good for.
    const IDS: &'static [(ProtocolVersion, i32)] = &[(ProtocolVersion::UNKNOWN, 0x00)];

    fn decode(r: &mut Reader<'_>, _version: ProtocolVersion) -> Result<Self> {
        Ok(Self {
            reason: r.string("reason", MAX_STATUS_LEN)?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _version: ProtocolVersion) -> Result<()> {
        w.string(&self.reason)
    }
}

/// Tells the client to reconnect to another server.
///
/// Does not exist before 1.20.5, which the ID table states outright.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfer {
    /// The target host.
    pub host: String,
    /// The target port.
    pub port: i32,
}

impl Packet for Transfer {
    const NAME: &'static str = "Transfer";
    const PHASE: Phase = Phase::Configuration;
    const DIRECTION: Direction = Direction::Clientbound;

    const IDS: &'static [(ProtocolVersion, i32)] = &[(versions::V1_20_5, 0x0B)];

    fn decode(r: &mut Reader<'_>, _version: ProtocolVersion) -> Result<Self> {
        Ok(Self {
            host: r.string("host", MAX_HOST_LEN)?,
            port: r.var_int()?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _version: ProtocolVersion) -> Result<()> {
        w.string(&self.host)?;
        w.var_int(self.port);
        Ok(())
    }
}

/// A keep-alive, sent by the server during the configuration phase.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct KeepAlive {
    /// The ID to be echoed by the client.
    pub id: i64,
}

impl Packet for KeepAlive {
    const NAME: &'static str = "KeepAlive";
    const PHASE: Phase = Phase::Configuration;
    const DIRECTION: Direction = Direction::Clientbound;

    const IDS: &'static [(ProtocolVersion, i32)] = &[(versions::V1_20_5, 0x04)];

    fn decode(r: &mut Reader<'_>, _version: ProtocolVersion) -> Result<Self> {
        Ok(Self { id: r.i64()? })
    }

    fn encode(&self, w: &mut Writer<'_>, _version: ProtocolVersion) -> Result<()> {
        w.i64(self.id);
        Ok(())
    }
}

/// The client's answer to a [`KeepAlive`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct KeepAliveResponse {
    /// The echoed ID.
    pub id: i64,
}

impl Packet for KeepAliveResponse {
    const NAME: &'static str = "KeepAliveResponse";
    const PHASE: Phase = Phase::Configuration;
    const DIRECTION: Direction = Direction::Serverbound;

    const IDS: &'static [(ProtocolVersion, i32)] = &[(versions::V1_20_5, 0x04)];

    fn decode(r: &mut Reader<'_>, _version: ProtocolVersion) -> Result<Self> {
        Ok(Self { id: r.i64()? })
    }

    fn encode(&self, w: &mut Writer<'_>, _version: ProtocolVersion) -> Result<()> {
        w.i64(self.id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::wire::Limits;
    use bytes::BytesMut;

    fn roundtrip<P: Packet + PartialEq + std::fmt::Debug>(
        packet: &P,
        version: ProtocolVersion,
    ) -> P {
        let mut buf = BytesMut::new();
        packet
            .encode(
                &mut Writer::new(&mut buf, P::NAME, Limits::default()),
                version,
            )
            .expect("encodes");
        let mut reader = Reader::new(&buf, Limits::default());
        let decoded = P::decode(&mut reader, version).expect("decodes");
        reader.finish(P::NAME).expect("consumes the whole payload");
        decoded
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

    /// Hand-written codecs can disagree with each other in a way generated ones could not, so this
    /// is the property that has to hold for every packet in every supported version.
    #[test]
    fn every_packet_roundtrips_in_every_version() {
        for version in [versions::V1_20_5, versions::V1_21, versions::V26_2] {
            let intention = Intention {
                protocol_version: version,
                server_address: "mc.justchunks.net".to_owned(),
                server_port: 25565,
                intent: Intent::Transfer,
            };
            assert_eq!(roundtrip(&intention, version), intention);
            assert_eq!(roundtrip(&StatusRequest, version), StatusRequest);
            assert_eq!(roundtrip(&LoginAcknowledged, version), LoginAcknowledged);

            let status = StatusResponse {
                body: r#"{"description":{"text":"hi"}}"#.to_owned(),
            };
            assert_eq!(roundtrip(&status, version), status);

            let ping = PingRequest { payload: -1 };
            assert_eq!(roundtrip(&ping, version), ping);
            let pong = PongResponse { payload: i64::MIN };
            assert_eq!(roundtrip(&pong, version), pong);

            let start = LoginStart {
                user_name: "Hydrofin".to_owned(),
                user_id: Uuid::from_u128(0x9999),
            };
            assert_eq!(roundtrip(&start, version), start);

            let transfer = Transfer {
                host: "backend-1.justchunks.net".to_owned(),
                port: 25565,
            };
            assert_eq!(roundtrip(&transfer, version), transfer);

            let keep_alive = KeepAlive { id: 7 };
            assert_eq!(roundtrip(&keep_alive, version), keep_alive);
            let response = KeepAliveResponse { id: 7 };
            assert_eq!(roundtrip(&response, version), response);

            // The gated field has to match what the version does with it.
            let success = login_success(
                version
                    .at_least(versions::V26_2)
                    .then(|| Uuid::from_u128(0x5678)),
            );
            assert_eq!(roundtrip(&success, version), success);
        }
    }

    #[test]
    fn the_gated_field_changes_the_encoding() {
        let packet = login_success(Some(Uuid::from_u128(0x5678)));
        let mut old = BytesMut::new();
        let mut new = BytesMut::new();
        packet
            .encode(
                &mut Writer::new(&mut old, "LoginSuccess", Limits::default()),
                versions::V1_21,
            )
            .expect("encodes");
        packet
            .encode(
                &mut Writer::new(&mut new, "LoginSuccess", Limits::default()),
                versions::V26_2,
            )
            .expect("encodes");
        // Exactly the 16 bytes of the session id.
        assert_eq!(new.len(), old.len() + 16);
    }

    #[test]
    fn a_missing_required_field_fails_closed() {
        // Sending a 26.2 client a packet without its session id would truncate the frame. The
        // encoder refuses instead of guessing a default.
        let err = login_success(None)
            .encode(
                &mut Writer::new(&mut BytesMut::new(), "LoginSuccess", Limits::default()),
                versions::V26_2,
            )
            .expect_err("must refuse");
        assert!(matches!(
            err,
            Error::Internal(InternalError::MissingField {
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
    fn an_invalid_intent_is_a_decode_error_not_a_handler_problem() {
        let mut buf = BytesMut::new();
        {
            let mut w = Writer::new(&mut buf, "Intention", Limits::default());
            w.var_int(versions::V1_21.get());
            w.string("mc.justchunks.net").expect("writes");
            w.u16(25565);
            w.var_int(9); // no such intent
        }
        let err = Intention::decode(&mut Reader::new(&buf, Limits::default()), versions::V1_21)
            .expect_err("must reject");
        assert_eq!(err.label(), "invalid_value");
    }

    #[test]
    fn an_overlong_hostname_is_rejected_by_the_field_limit() {
        // Well under the crate-wide string backstop, and still refused: the field says 255.
        let mut buf = BytesMut::new();
        {
            let mut w = Writer::new(&mut buf, "Intention", Limits::default());
            w.var_int(versions::V1_21.get());
            w.string(&"a".repeat(300)).expect("writes");
            w.u16(25565);
            w.var_int(1);
        }
        let err = Intention::decode(&mut Reader::new(&buf, Limits::default()), versions::V1_21)
            .expect_err("must reject");
        assert_eq!(err.label(), "length_limit");
    }
}
