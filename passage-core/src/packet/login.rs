//! The packets of the login phase, which is where the encryption handshake, the game profile and
//! the login cookies happen. Everything here is anchored at 1.20.5, the oldest version whose IDs
//! and fields these are.
//!
//! One thing sets the phase apart from [`configuration`](crate::packet::configuration): a
//! disconnect reason is still the JSON the client parsed before 1.20.3, not NBT. The client has not
//! been told a codec yet, so there is nothing here to send NBT against.

use crate::common::{
    MAX_COMPONENT_JSON_LEN, MAX_COOKIE_LEN, MAX_CRYPTO_BLOB_LEN, MAX_IDENTIFIER_LEN,
    MAX_PROFILE_PROPERTIES, MAX_SERVER_ID_LEN, MAX_USERNAME_LEN, Profile, ProfileProperty, VarInt,
    versions,
};
use crate::wire::{Reader, WireError, Writer};
use crate::{Packet, Phase, ProtocolVersion};
use bytes::Bytes;
use bytestring::ByteString;
use uuid::Uuid;

/// The [`ServerDisconnectPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Disconnect_(login))
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerDisconnectPacket {
    /// The JSON text component explaining why the player was turned away.
    pub reason: ByteString,
}

impl ServerDisconnectPacket {
    /// Creates a new [`ServerDisconnectPacket`] from literal text, wrapped in the JSON object the
    /// login phase expects.
    #[must_use]
    pub fn text(reason: impl Into<String>) -> Self {
        // `Value`'s `Display` is its JSON, so the text is escaped by the same code that would write
        // it as part of a larger document -- and a string cannot fail to serialise.
        let text = serde_json::Value::String(reason.into());
        Self {
            reason: format!(r#"{{"text":{text}}}"#).into(),
        }
    }
}

impl Packet for ServerDisconnectPacket {
    // The configuration phase has a disconnect of its own, and a name is what a trace and a metric
    // tell the two apart by.
    const NAME: &'static str = "server::login_disconnect";
    const PHASE: Phase = Phase::Login;
    // Anchored at the floor: the one packet that has to be sendable to a client whose version
    // resolves nothing else, because the message it carries is why that client is being turned
    // away. Its ID and its JSON reason have been the same since the phase existed.
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(ProtocolVersion::UNKNOWN, 0x00)];

    fn decode(r: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            reason: r.string("reason", MAX_COMPONENT_JSON_LEN)?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.string("reason", &self.reason)?;
        Ok(())
    }
}

/// The [`ServerEncryptionRequestPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Encryption_Request)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerEncryptionRequestPacket {
    /// The server ID that goes into the session hash, which is empty for every server since 1.7.
    pub server_id: ByteString,
    /// The DER-encoded public key the client encrypts the shared secret under.
    pub public_key: Bytes,
    /// The token the client has to return encrypted, proving it holds the shared secret.
    pub verify_token: Bytes,
    /// Whether the client authenticates against the session server before answering.
    pub should_authenticate: bool,
}

impl Packet for ServerEncryptionRequestPacket {
    const NAME: &'static str = "server::encryption_request";
    const PHASE: Phase = Phase::Login;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x01)];

    fn decode(r: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            server_id: r.string("server_id", MAX_SERVER_ID_LEN)?,
            public_key: r.bytes("public_key", MAX_CRYPTO_BLOB_LEN)?,
            verify_token: r.bytes("verify_token", MAX_CRYPTO_BLOB_LEN)?,
            should_authenticate: r.bool("should_authenticate")?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.string("server_id", &self.server_id)?;
        w.bytes("public_key", &self.public_key)?;
        w.bytes("verify_token", &self.verify_token)?;
        w.bool(self.should_authenticate);
        Ok(())
    }
}

/// The [`ServerLoginSuccessPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Login_Success)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerLoginSuccessPacket {
    /// The unique identifier of the player, which the client takes as its own from here on.
    pub uuid: Uuid,
    /// The name of the player.
    pub name: ByteString,
    /// The signed properties of the profile, such as `textures`.
    pub properties: Vec<ProfileProperty>,
    /// Whether the client disconnects on any packet error, between 1.20.5 and 1.21.2. The vanilla
    /// server sends `true`, which is what an absent value is written as.
    pub strict_error_handling: Option<bool>,
    /// The session of the player, from 26.2 on. It identifies the sitting of the server rather
    /// than the player: the vanilla server generates one and hands the same value to everybody
    /// until it empties out.
    pub session_id: Option<Uuid>,
}

impl From<&Profile> for ServerLoginSuccessPacket {
    fn from(profile: &Profile) -> Self {
        Self {
            uuid: profile.id,
            name: profile.name.clone(),
            properties: profile.properties.clone(),
            strict_error_handling: None,
            session_id: None,
        }
    }
}

impl Packet for ServerLoginSuccessPacket {
    const NAME: &'static str = "server::login_success";
    const PHASE: Phase = Phase::Login;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x02)];

    fn decode(r: &mut Reader, version: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            uuid: r.uuid("uuid")?,
            name: r.string("name", MAX_USERNAME_LEN)?,
            properties: r.array("properties", MAX_PROFILE_PROPERTIES, |r| {
                r.property(version, "property")
            })?,
            strict_error_handling: r.gated(!version.at_least(versions::V1_21_2), |r| {
                r.bool("strict_error_handling")
            })?,
            session_id: r.gated(version.at_least(versions::V26_2), |r| r.uuid("session_id"))?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, version: ProtocolVersion) -> Result<(), WireError> {
        w.uuid(&self.uuid);
        w.string("name", &self.name)?;
        w.array("properties", &self.properties, |w, property| {
            w.property(version, "property", property)
        })?;
        if !version.at_least(versions::V1_21_2) {
            w.bool(self.strict_error_handling.unwrap_or(true));
        }
        if version.at_least(versions::V26_2) {
            // A player Passage transfers spends no time on a server of ours, so there is no sitting
            // to name -- and the client only forwards the value in its telemetry.
            w.uuid(&self.session_id.unwrap_or(Uuid::nil()));
        }
        Ok(())
    }
}

/// The [`ServerSetCompressionPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Set_Compression)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerSetCompressionPacket {
    /// The size from which a packet is compressed. A negative threshold disables compression, which
    /// is also what not sending the packet at all means.
    pub threshold: VarInt,
}

impl Packet for ServerSetCompressionPacket {
    const NAME: &'static str = "server::set_compression";
    const PHASE: Phase = Phase::Login;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x03)];

    fn decode(r: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            threshold: r.var_int("threshold")?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.var_int(self.threshold);
        Ok(())
    }
}

/// The [`ServerLoginPluginRequestPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Login_Plugin_Request)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerLoginPluginRequestPacket {
    /// The ID the client's answer refers back to.
    pub message_id: VarInt,
    /// The plugin channel the data belongs to.
    pub channel: ByteString,
    /// The channel-specific data, which runs to the end of the packet.
    pub data: Bytes,
}

impl Packet for ServerLoginPluginRequestPacket {
    const NAME: &'static str = "server::login_plugin_request";
    const PHASE: Phase = Phase::Login;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x04)];

    fn decode(r: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            message_id: r.var_int("message_id")?,
            channel: r.string("channel", MAX_IDENTIFIER_LEN)?,
            data: r.pop_rest(),
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.var_int(self.message_id);
        w.string("channel", &self.channel)?;
        w.raw(&self.data);
        Ok(())
    }
}

/// The [`ServerCookieRequestPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Cookie_Request_(login))
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ServerCookieRequestPacket {
    /// The identifier of the cookie.
    pub key: ByteString,
}

impl Packet for ServerCookieRequestPacket {
    const NAME: &'static str = "server::login_cookie_request";
    const PHASE: Phase = Phase::Login;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x05)];

    fn decode(r: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            key: r.string("key", MAX_IDENTIFIER_LEN)?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.string("key", &self.key)?;
        Ok(())
    }
}

/// The [`ClientLoginStartPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Login_Start)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClientLoginStartPacket {
    /// The name the player claims, which is only theirs once the session server says so.
    pub name: ByteString,
    /// The unique identifier the player claims, under the same caveat.
    pub uuid: Uuid,
}

impl Packet for ClientLoginStartPacket {
    const NAME: &'static str = "client::login_start";
    const PHASE: Phase = Phase::Login;
    // Anchored at the floor, so that a client Passage cannot transfer still reaches a handler and
    // can be told so. The two fields have been these two since 1.20.2; older clients send a shape
    // this does not decode, and are refused for that instead of for having no ID.
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(ProtocolVersion::UNKNOWN, 0x00)];

    fn decode(r: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            name: r.string("name", MAX_USERNAME_LEN)?,
            uuid: r.uuid("uuid")?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.string("name", &self.name)?;
        w.uuid(&self.uuid);
        Ok(())
    }
}

/// The [`ClientEncryptionResponsePacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Encryption_Response)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClientEncryptionResponsePacket {
    /// The shared secret, encrypted under the public key of the request.
    pub shared_secret: Bytes,
    /// The verify token of the request, encrypted under the same key.
    pub verify_token: Bytes,
}

impl Packet for ClientEncryptionResponsePacket {
    const NAME: &'static str = "client::encryption_response";
    const PHASE: Phase = Phase::Login;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x01)];

    fn decode(r: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            shared_secret: r.bytes("shared_secret", MAX_CRYPTO_BLOB_LEN)?,
            verify_token: r.bytes("verify_token", MAX_CRYPTO_BLOB_LEN)?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.bytes("shared_secret", &self.shared_secret)?;
        w.bytes("verify_token", &self.verify_token)?;
        Ok(())
    }
}

/// The [`ClientLoginPluginResponsePacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Login_Plugin_Response)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClientLoginPluginResponsePacket {
    /// The ID of the request this answers.
    pub message_id: VarInt,
    /// The answer, which runs to the end of the packet, and is absent if the client did not
    /// understand the channel -- which is what the vanilla client always says.
    pub data: Option<Bytes>,
}

impl Packet for ClientLoginPluginResponsePacket {
    const NAME: &'static str = "client::login_plugin_response";
    const PHASE: Phase = Phase::Login;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x02)];

    fn decode(r: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            message_id: r.var_int("message_id")?,
            data: r.optional("data", |r| Ok(r.pop_rest()))?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.var_int(self.message_id);
        w.optional(self.data.as_deref(), |w, data| {
            w.raw(data);
            Ok(())
        })?;
        Ok(())
    }
}

/// The [`ClientLoginAcknowledgedPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Login_Acknowledged)
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClientLoginAcknowledgedPacket;

impl Packet for ClientLoginAcknowledgedPacket {
    const NAME: &'static str = "client::login_acknowledged";
    const PHASE: Phase = Phase::Login;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x03)];

    fn decode(_: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self)
    }

    fn encode(&self, _: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        Ok(())
    }
}

/// The [`ClientCookieResponsePacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol/Packets#Cookie_Response_(login))
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClientCookieResponsePacket {
    /// The identifier of the cookie.
    pub key: ByteString,
    /// The data of the cookie, absent if the client has none stored.
    pub payload: Option<Bytes>,
}

impl Packet for ClientCookieResponsePacket {
    const NAME: &'static str = "client::login_cookie_response";
    const PHASE: Phase = Phase::Login;
    const IDS: &'static [(ProtocolVersion, VarInt)] = &[(versions::V1_20_5, 0x04)];

    fn decode(r: &mut Reader, _: ProtocolVersion) -> Result<Self, WireError> {
        Ok(Self {
            key: r.string("key", MAX_IDENTIFIER_LEN)?,
            payload: r.optional("payload", |r| r.bytes("payload", MAX_COOKIE_LEN))?,
        })
    }

    fn encode(&self, w: &mut Writer<'_>, _: ProtocolVersion) -> Result<(), WireError> {
        w.string("key", &self.key)?;
        w.optional(self.payload.as_deref(), |w, payload| {
            w.bytes("payload", payload)
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::BytesMut;

    /// Round trips a packet at `version` through the writer and the reader it will meet.
    fn round_trip<P: Packet + std::fmt::Debug>(packet: &P, version: ProtocolVersion) -> P {
        let mut buf = BytesMut::new();
        packet
            .encode(&mut Writer::new(&mut buf), version)
            .expect("encodes");
        let mut r = Reader::new(buf.clone().freeze());
        let decoded = P::decode(&mut r, version).expect("decodes");
        r.finish(P::NAME).expect("consumes the whole payload");
        decoded
    }

    /// A profile as the session server hands it over: one signed property.
    fn profile() -> Profile {
        Profile {
            id: Uuid::from_u128(1),
            name: "Notch".into(),
            properties: vec![ProfileProperty {
                name: "textures".into(),
                value: "eyJ0aW1lc3RhbXAiOjF9".into(),
                signature: Some("c2lnbmF0dXJl".into()),
            }],
            profile_actions: vec![],
        }
    }

    #[test]
    fn the_packets_a_login_needs_round_trip() {
        // The exchange in the order a player meets it: the cookie Passage asks for, the encryption
        // handshake, and the profile that ends the phase.
        let packet = ClientLoginStartPacket {
            name: "Notch".into(),
            uuid: Uuid::from_u128(1),
        };
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);

        let packet = ServerCookieRequestPacket {
            key: "justchunks:session".into(),
        };
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);

        let packet = ServerEncryptionRequestPacket {
            server_id: ByteString::new(),
            public_key: Bytes::from_static(&[0x30, 0x81, 0x9F]),
            verify_token: Bytes::from(vec![0xAA; 32]),
            should_authenticate: true,
        };
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);

        let packet = ClientEncryptionResponsePacket {
            shared_secret: Bytes::from(vec![0x01; 128]),
            verify_token: Bytes::from(vec![0x02; 128]),
        };
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);

        let packet = ClientLoginAcknowledgedPacket;
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);

        let packet = ServerDisconnectPacket::text("Server full");
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);
    }

    #[test]
    fn a_login_disconnect_is_json_rather_than_nbt() {
        // The one packet whose shape differs from its configuration namesake: the client has not
        // been told a codec yet, so the reason is the JSON it parsed before 1.20.3 -- and the text
        // has to survive the quotes a player can put in a kick message.
        let packet = ServerDisconnectPacket::text(r#"Go "away""#);
        assert_eq!(packet.reason, r#"{"text":"Go \"away\""}"#);
        let value: serde_json::Value = serde_json::from_str(&packet.reason).expect("is JSON");
        assert_eq!(value["text"], r#"Go "away""#);
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);
    }

    #[test]
    fn login_success_carries_the_profile_it_was_built_from() {
        let profile = profile();
        let packet = ServerLoginSuccessPacket::from(&profile);
        assert_eq!(packet.uuid, profile.id);
        assert_eq!(packet.name, profile.name);
        assert_eq!(packet.properties, profile.properties);

        // An unsigned property costs the byte that says so, and nothing more.
        let decoded = round_trip(&packet, versions::V1_21_2);
        assert_eq!(decoded.properties, profile.properties);
        let packet = ServerLoginSuccessPacket {
            properties: vec![ProfileProperty {
                name: "textures".into(),
                value: "eyJ0aW1lc3RhbXAiOjF9".into(),
                signature: None,
            }],
            ..packet
        };
        assert_eq!(round_trip(&packet, versions::V1_21_2), packet);
    }

    #[test]
    fn login_success_loses_a_field_at_1_21_2_and_gains_one_at_26_2() {
        // Two thresholds in one packet, in opposite directions: a client that is sent the wrong
        // set of them reads the profile of whoever logs in next.
        let packet = ServerLoginSuccessPacket {
            strict_error_handling: Some(true),
            session_id: Some(Uuid::from_u128(2)),
            ..ServerLoginSuccessPacket::from(&profile())
        };
        assert_eq!(
            round_trip(&packet, versions::V1_20_5),
            ServerLoginSuccessPacket {
                session_id: None,
                ..packet.clone()
            },
            "1.20.5 has strict error handling and no session",
        );
        assert_eq!(
            round_trip(&packet, versions::V1_21_2),
            ServerLoginSuccessPacket {
                strict_error_handling: None,
                session_id: None,
                ..packet.clone()
            },
            "1.21.2 dropped strict error handling",
        );
        assert_eq!(
            round_trip(&packet, versions::V26_2),
            ServerLoginSuccessPacket {
                strict_error_handling: None,
                ..packet.clone()
            },
            "26.2 added the session",
        );

        // And what an absent value is written as, for each of the two.
        let packet = ServerLoginSuccessPacket::from(&profile());
        assert_eq!(
            round_trip(&packet, versions::V1_20_5).strict_error_handling,
            Some(true),
            "the vanilla server sends `true`",
        );
        assert_eq!(
            round_trip(&packet, versions::V26_2).session_id,
            Some(Uuid::nil()),
        );
    }

    #[test]
    fn login_success_is_the_length_the_vanilla_server_sends() {
        // Byte-for-byte against what the official server actually put on the socket for the same
        // profile: 26.1 ends after the (empty) property array, 26.2 carries sixteen bytes more.
        // The lengths were taken by logging into a vanilla 26.1 and 26.2 server and counting the
        // payload, which is the only way to settle a field that is absent in one version and
        // present in the next -- the encoder and the decoder here would agree either way.
        let packet = ServerLoginSuccessPacket {
            uuid: Uuid::from_u128(1),
            name: "Probe".into(),
            properties: vec![],
            strict_error_handling: None,
            session_id: None,
        };
        let encode = |version| {
            let mut buf = BytesMut::new();
            packet
                .encode(&mut Writer::new(&mut buf), version)
                .expect("encodes");
            buf.len()
        };
        // 16 (uuid) + 1 (name length) + 5 (name) + 1 (property count).
        assert_eq!(encode(ProtocolVersion::new(775)), 23, "26.1");
        assert_eq!(encode(versions::V26_2), 23 + 16, "26.2 adds the session");
        assert_eq!(encode(versions::V26_3), 23 + 16, "26.3 keeps it");
        // 1.20.5 has no session, but it does have the strict error handling flag.
        assert_eq!(encode(versions::V1_20_5), 24, "1.20.5");
    }

    #[test]
    fn a_plugin_answer_the_client_did_not_understand_costs_one_byte() {
        // The vanilla client answers every login plugin request this way, so it is the path that
        // actually runs -- and the data has no length of its own, only the frame's.
        let packet = ClientLoginPluginResponsePacket {
            message_id: 1,
            data: None,
        };
        let mut buf = BytesMut::new();
        packet
            .encode(&mut Writer::new(&mut buf), versions::V1_20_5)
            .expect("encodes");
        assert_eq!(buf.as_ref(), &[0x01, 0x00]);
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);

        let packet = ClientLoginPluginResponsePacket {
            message_id: 1,
            data: Some(Bytes::from_static(b"justchunks")),
        };
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);

        let packet = ServerLoginPluginRequestPacket {
            message_id: 1,
            channel: "justchunks:queue".into(),
            data: Bytes::from_static(b"justchunks"),
        };
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);
    }

    #[test]
    fn a_cookie_the_client_does_not_have_costs_one_byte() {
        let packet = ClientCookieResponsePacket {
            key: "justchunks:session".into(),
            payload: None,
        };
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);

        let packet = ClientCookieResponsePacket {
            key: "justchunks:session".into(),
            payload: Some(Bytes::from_static(&[0x01, 0x02, 0x03])),
        };
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);
    }

    #[test]
    fn compression_is_a_threshold_that_may_be_negative() {
        // The disabling value, which a plain length prefix would have refused.
        let packet = ServerSetCompressionPacket { threshold: -1 };
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);

        let packet = ServerSetCompressionPacket { threshold: 256 };
        assert_eq!(round_trip(&packet, versions::V1_20_5), packet);
    }

    #[test]
    fn the_phase_does_not_exist_below_the_version_passage_speaks() {
        // 1.20.4. Below 1.20.5 there is no cookie to ask for and no transfer to follow, so the
        // phase resolves to nothing rather than to IDs a client would read as other packets.
        const V1_20_4: ProtocolVersion = ProtocolVersion::new(765);
        assert_eq!(ServerCookieRequestPacket::id(V1_20_4), None);
        assert_eq!(ServerCookieRequestPacket::id(versions::V1_20_5), Some(0x05));
        assert_eq!(ServerLoginSuccessPacket::id(V1_20_4), None);

        // The two exceptions, which is what lets such a client be turned away with a reason rather
        // than dropped: it says who it is, and it is told which version to install.
        assert_eq!(ClientLoginStartPacket::id(V1_20_4), Some(0x00));
        assert_eq!(ServerDisconnectPacket::id(V1_20_4), Some(0x00));
        // Including a version nothing can place, which is what a snapshot resolves as.
        let snapshot = ProtocolVersion::new(0x4000_0000 | 132);
        assert_eq!(ClientLoginStartPacket::id(snapshot), Some(0x00));
        assert_eq!(ServerDisconnectPacket::id(snapshot), Some(0x00));
        assert_eq!(ServerLoginSuccessPacket::id(snapshot), None);

        // And the IDs are stable from there on, in both directions. The login phase is the one
        // Passage speaks whose IDs have not moved once since 1.20.5.
        assert_eq!(ServerLoginSuccessPacket::id(versions::V26_3), Some(0x02));
        assert_eq!(ClientCookieResponsePacket::id(versions::V26_3), Some(0x04));
    }
}
