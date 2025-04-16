use crate::authentication;
use crate::authentication::VerifyToken;
use crate::connection::KeepAlive;
use crate::connection::{Connection, Phase, phase};
use crate::protocol::configuration::outbound::{AddResourcePackPacket, StoreCookiePacket};
use crate::protocol::{
    AsyncReadPacket, AsyncWritePacket, Error, InboundPacket, OutboundPacket, Packet,
};
use crate::status::Protocol;
use fake::Dummy;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::{debug, warn};
use uuid::Uuid;

pub const AUTH_COOKIE_KEY: &str = "passage:authentication";
pub const AUTH_COOKIE_EXPIRY_SECS: u64 = 6 * 60 * 60; // 6 hours

#[derive(Debug, Serialize, Deserialize)]
pub struct AuthCookie {
    pub timestamp: u64,
    pub client_addr: SocketAddr,
    pub user_name: String,
    pub user_id: Uuid,
}

pub mod outbound {
    use super::*;

    /// The outbound [`DisconnectPacket`]. (Placeholder)
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Disconnect_(login))
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct DisconnectPacket {
        /// The JSON text component containing the reason of the disconnect.
        pub(crate) reason: String,
    }

    impl Packet for DisconnectPacket {
        fn get_packet_id() -> usize {
            0x00
        }
    }

    impl OutboundPacket for DisconnectPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_string(&self.reason).await?;

            Ok(())
        }
    }

    #[cfg(test)]
    impl InboundPacket for DisconnectPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let reason = buffer.read_string().await?;

            Ok(Self { reason })
        }
    }

    /// The outbound [`EncryptionRequestPacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Encryption_Request)
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct EncryptionRequestPacket {
        // ignore max size
        pub server_id: String,
        pub public_key: Vec<u8>,
        pub verify_token: VerifyToken,
        pub should_authenticate: bool,
    }

    impl Packet for EncryptionRequestPacket {
        fn get_packet_id() -> usize {
            0x01
        }
    }

    impl OutboundPacket for EncryptionRequestPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_string(&self.server_id).await?;
            buffer.write_bytes(&self.public_key).await?;
            buffer.write_bytes(&self.verify_token).await?;
            buffer.write_bool(self.should_authenticate).await?;

            Ok(())
        }
    }

    #[cfg(test)]
    impl InboundPacket for EncryptionRequestPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let server_id = buffer.read_string().await?;
            let public_key = buffer.read_bytes().await?;
            let verify_token = buffer
                .read_bytes()
                .await?
                .try_into()
                .map_err(|_| Error::ArrayConversionFailed)?;
            let should_authenticate = buffer.read_bool().await?;

            Ok(Self {
                server_id,
                public_key,
                verify_token,
                should_authenticate,
            })
        }
    }

    /// The outbound [`LoginSuccessPacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Login_Success)
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct LoginSuccessPacket {
        pub user_id: Uuid,
        pub user_name: String,
        // properties - we don't need those
    }

    impl Packet for LoginSuccessPacket {
        fn get_packet_id() -> usize {
            0x02
        }
    }

    impl OutboundPacket for LoginSuccessPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_uuid(&self.user_id).await?;
            buffer.write_string(&self.user_name).await?;
            // no properties in array
            buffer.write_varint(0).await?;

            Ok(())
        }
    }

    #[cfg(test)]
    impl InboundPacket for LoginSuccessPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            println!(".");
            let user_id = buffer.read_uuid().await?;
            println!(".");
            let user_name = buffer.read_string().await?;
            println!(".");
            // expect no properties in array
            let _properties = buffer.read_varint().await?;
            println!(".");

            Ok(Self { user_id, user_name })
        }
    }

    /// The outbound [`SetCompressionPacket`]. (Placeholder)
    ///
    /// Enables compression. If compression is enabled, all following packets are encoded in the compressed
    /// packet format. Negative values will disable compression, meaning the packet format should remain
    /// in the uncompressed packet format. However, this packet is entirely optional, and if not sent,
    /// compression will also not be enabled (the vanilla server does not send the packet when compression
    /// is disabled).
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Set_Compression)
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct SetCompressionPacket;

    impl Packet for SetCompressionPacket {
        fn get_packet_id() -> usize {
            0x03
        }
    }

    impl OutboundPacket for SetCompressionPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(test)]
    impl InboundPacket for SetCompressionPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The outbound [`LoginPluginRequestPacket`]. (Placeholder)
    ///
    /// Used to implement a custom handshaking flow together with Login Plugin Response. Unlike plugin
    /// messages in "play" mode, these messages follow a lock-step request/response scheme, where the
    /// client is expected to respond to a request indicating whether it understood. The vanilla client
    /// always responds that it hasn't understood, and sends an empty payload.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Login_Plugin_Request)
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct LoginPluginRequestPacket;

    impl Packet for LoginPluginRequestPacket {
        fn get_packet_id() -> usize {
            0x04
        }
    }

    impl OutboundPacket for LoginPluginRequestPacket {
        async fn write_to_buffer<S>(&self, _buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    #[cfg(test)]
    impl InboundPacket for LoginPluginRequestPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The outbound [`CookieRequestPacket`]. (Placeholder)
    ///
    /// Requests a cookie that was previously stored.
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Cookie_Request_(login))
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct CookieRequestPacket {
        pub key: String,
    }

    impl Packet for CookieRequestPacket {
        fn get_packet_id() -> usize {
            0x05
        }
    }

    impl OutboundPacket for CookieRequestPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_string(&self.key).await?;

            Ok(())
        }
    }

    #[cfg(test)]
    impl InboundPacket for CookieRequestPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let key = buffer.read_string().await?;

            Ok(Self { key })
        }
    }
}

pub mod inbound {
    use super::*;
    use crate::protocol::login::outbound::CookieRequestPacket;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// The inbound [`LoginStartPacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Login_Start)
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct LoginStartPacket {
        pub user_name: String,
        pub user_id: Uuid,
    }

    impl Packet for LoginStartPacket {
        fn get_packet_id() -> usize {
            0x00
        }
    }

    #[cfg(test)]
    impl OutboundPacket for LoginStartPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_string(&self.user_name).await?;
            buffer.write_uuid(&self.user_id).await?;

            Ok(())
        }
    }

    impl InboundPacket for LoginStartPacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let name = buffer.read_string().await?;
            let user_id = buffer.read_uuid().await?;

            Ok(Self {
                user_name: name,
                user_id,
            })
        }

        async fn handle<S>(self, con: &mut Connection<S>) -> Result<(), Error>
        where
            S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
        {
            debug!(packet = debug(&self), "received login start packet");
            phase!(
                con.phase,
                Phase::Login,
                client_address,
                protocol_version,
                server_address,
                server_port,
                transfer,
            );

            // handle transfer
            if *transfer && con.auth_secret.is_some() {
                // update phase and wait for cookie
                con.phase = Phase::Transfer {
                    client_address: *client_address,
                    protocol_version: *protocol_version,
                    server_address: server_address.clone(),
                    server_port: *server_port,
                    user_name: self.user_name.clone(),
                    user_id: self.user_id,
                };
                con.write_packet(CookieRequestPacket {
                    key: AUTH_COOKIE_KEY.to_string(),
                })
                .await?;
                return Ok(());
            }

            // encode public key and generate verify token
            let verify_token = authentication::generate_token()?;

            // switch phase to accept encryption response
            con.phase = Phase::Encryption {
                client_address: *client_address,
                protocol_version: *protocol_version,
                server_address: server_address.clone(),
                server_port: *server_port,
                user_name: self.user_name.clone(),
                user_id: self.user_id,
                verify_token,
                should_authenticate: true,
            };

            // create a new encryption request and send it
            let encryption_request = outbound::EncryptionRequestPacket {
                server_id: "".to_owned(),
                public_key: authentication::ENCODED_PUB.clone(),
                verify_token,
                should_authenticate: true,
            };
            debug!(
                packet = debug(&encryption_request),
                "sending encryption request packet"
            );
            con.write_packet(encryption_request).await?;

            Ok(())
        }
    }

    /// The inbound [`EncryptionResponsePacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Encryption_Response)
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct EncryptionResponsePacket {
        pub shared_secret: Vec<u8>,
        pub verify_token: Vec<u8>,
    }

    impl Packet for EncryptionResponsePacket {
        fn get_packet_id() -> usize {
            0x01
        }
    }

    #[cfg(test)]
    impl OutboundPacket for EncryptionResponsePacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_bytes(&self.shared_secret).await?;
            buffer.write_bytes(&self.verify_token).await?;

            Ok(())
        }
    }

    impl InboundPacket for EncryptionResponsePacket {
        async fn new_from_buffer<S>(reader: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let shared_secret = reader.read_bytes().await?;
            let verify_token = reader.read_bytes().await?;

            Ok(Self {
                shared_secret,
                verify_token,
            })
        }

        async fn handle<S>(self, con: &mut Connection<S>) -> Result<(), Error>
        where
            S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
        {
            debug!(packet = debug(&self), "received encryption response packet");
            phase!(
                con.phase,
                Phase::Encryption,
                client_address,
                protocol_version,
                server_address,
                server_port,
                user_name,
                user_id,
                verify_token,
                should_authenticate,
            );

            // decrypt the shared secret and verify token
            let shared_secret =
                authentication::decrypt(&authentication::KEY_PAIR.0, &self.shared_secret)?;
            let decrypted_verify_token =
                authentication::decrypt(&authentication::KEY_PAIR.0, &self.verify_token)?;

            // verify the token is correct
            authentication::verify_token(*verify_token, &decrypted_verify_token)?;

            // handle mojang auth if not handled by cookie
            if *should_authenticate {
                // get the data for login success
                let auth_response = authentication::authenticate_mojang(
                    user_name,
                    &shared_secret,
                    &authentication::ENCODED_PUB,
                )
                .await;

                let auth_response = match auth_response {
                    Ok(inner) => inner,
                    Err(err) => {
                        warn!(err = ?err, "mojang auth failed");
                        // TODO write actual reason
                        con.write_packet(outbound::DisconnectPacket {
                            reason: "".to_string(),
                        })
                        .await?;
                        con.shutdown();
                        return Ok(());
                    }
                };

                // update state for actual use info
                *user_name = auth_response.name;
                *user_id = auth_response.id;
            }

            // build response packet
            let login_success = outbound::LoginSuccessPacket {
                user_name: user_name.clone(),
                user_id: *user_id,
            };

            // switch to login-acknowledge phase
            con.phase = Phase::Acknowledge {
                client_address: *client_address,
                protocol_version: *protocol_version,
                server_address: server_address.clone(),
                server_port: *server_port,
                user_name: user_name.clone(),
                user_id: *user_id,
                should_write_auth_cookie: *should_authenticate,
            };

            // enable encryption for the connection using the shared secret
            con.apply_encryption(&shared_secret)?;

            // create a new login success packet and send it
            debug!(
                packet = debug(&login_success),
                "sending login success packet"
            );
            con.write_packet(login_success).await?;

            Ok(())
        }
    }

    /// The inbound [`LoginPluginResponsePacket`]. (Placeholder)
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Login_Plugin_Response)
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct LoginPluginResponsePacket;

    impl Packet for LoginPluginResponsePacket {
        fn get_packet_id() -> usize {
            0x02
        }
    }

    #[cfg(test)]
    impl OutboundPacket for LoginPluginResponsePacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    impl InboundPacket for LoginPluginResponsePacket {
        async fn new_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }
    }

    /// The inbound [`LoginAcknowledgedPacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Login_Acknowledged)
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct LoginAcknowledgedPacket;

    impl Packet for LoginAcknowledgedPacket {
        fn get_packet_id() -> usize {
            0x03
        }
    }

    #[cfg(test)]
    impl OutboundPacket for LoginAcknowledgedPacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            Ok(())
        }
    }

    impl InboundPacket for LoginAcknowledgedPacket {
        async fn new_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            Ok(Self)
        }

        async fn handle<S>(self, con: &mut Connection<S>) -> Result<(), Error>
        where
            S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
        {
            debug!(packet = debug(&self), "received login acknowledged packet");
            phase!(
                con.phase,
                Phase::Acknowledge,
                client_address,
                protocol_version,
                server_address,
                server_port,
                user_name,
                user_id,
                should_write_auth_cookie,
            );
            let should_write_auth_cookie = *should_write_auth_cookie;

            // generate auth cookie payload
            let now_secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time error")
                .as_secs();
            let auth_payload = serde_json::to_vec(&AuthCookie {
                timestamp: now_secs,
                client_addr: *client_address,
                user_name: user_name.clone(),
                user_id: *user_id,
            })?;

            // get resource packs to load
            let packs = con
                .resourcepack_supplier
                .get_resourcepacks(
                    client_address,
                    (server_address, *server_port),
                    *protocol_version as Protocol,
                    user_name,
                    user_id,
                )
                .await?;
            let pack_ids = packs.iter().map(|pack| (pack.uuid, pack.forced)).collect();

            // switch to configuration phase
            con.phase = Phase::Configuration {
                client_address: *client_address,
                protocol_version: *protocol_version,
                server_address: server_address.clone(),
                server_port: *server_port,
                user_name: user_name.clone(),
                user_id: *user_id,
                transit_packs: pack_ids,
                last_keep_alive: KeepAlive::empty(),
            };

            // store auth cookie
            if should_write_auth_cookie {
                if let Some(secret) = &con.auth_secret {
                    con.write_packet(StoreCookiePacket {
                        key: AUTH_COOKIE_KEY.to_string(),
                        payload: authentication::sign(&auth_payload, secret),
                    })
                    .await?;
                }
            }

            // handle no resource packs to send
            if packs.is_empty() {
                return con.transfer().await;
            }

            // send resource packs
            for pack in packs {
                let packet = AddResourcePackPacket {
                    uuid: pack.uuid,
                    url: pack.url,
                    hash: pack.hash,
                    forced: pack.forced,
                    prompt_message: pack.prompt_message,
                };
                con.write_packet(packet).await?;
            }

            Ok(())
        }
    }

    /// The inbound [`CookieResponsePacket`].
    ///
    /// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Cookie_Response_(login))
    #[derive(Debug, Clone, PartialEq, Eq, Dummy)]
    pub struct CookieResponsePacket {
        pub key: String,
        pub payload: Option<Vec<u8>>,
    }

    impl Packet for CookieResponsePacket {
        fn get_packet_id() -> usize {
            0x04
        }
    }

    #[cfg(test)]
    impl OutboundPacket for CookieResponsePacket {
        async fn write_to_buffer<S>(&self, buffer: &mut S) -> Result<(), Error>
        where
            S: AsyncWrite + Unpin + Send + Sync,
        {
            buffer.write_string(&self.key).await?;
            buffer.write_bool(self.payload.is_some()).await?;
            if let Some(payload) = &self.payload {
                buffer.write_bytes(payload).await?;
            }

            Ok(())
        }
    }

    impl InboundPacket for CookieResponsePacket {
        async fn new_from_buffer<S>(buffer: &mut S) -> Result<Self, Error>
        where
            S: AsyncRead + Unpin + Send + Sync,
        {
            let key = buffer.read_string().await?;
            let has_payload = buffer.read_bool().await?;
            let mut payload = None;
            if has_payload {
                payload = Some(buffer.read_bytes().await?);
            }

            Ok(Self { key, payload })
        }

        async fn handle<S>(self, con: &mut Connection<S>) -> Result<(), Error>
        where
            S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
        {
            debug!(packet = debug(&self), "received cookie response packet");

            // only supports auth cookie
            if self.key != AUTH_COOKIE_KEY {
                return Ok(());
            }

            // get auth cookie secret
            let Some(secret) = &con.auth_secret else {
                return Ok(());
            };

            phase!(
                con.phase,
                Phase::Transfer,
                client_address,
                protocol_version,
                server_address,
                server_port,
                user_name,
                user_id,
            );

            // verify token
            let mut should_authenticate = true;
            if let Some(message) = self.payload {
                let (ok, message) = authentication::check_sign(&message, secret);
                if ok {
                    let cookie = serde_json::from_slice::<AuthCookie>(message)?;
                    let expires_at = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("time error")
                        .as_secs()
                        + AUTH_COOKIE_EXPIRY_SECS;
                    if cookie.client_addr.ip() == client_address.ip()
                        && cookie.timestamp < expires_at
                    {
                        should_authenticate = false;

                        // update state by token
                        *user_name = cookie.user_name;
                        *user_id = cookie.user_id;
                    }
                }
            }

            // encode public key and generate verify token
            let verify_token = authentication::generate_token()?;

            // switch phase to accept encryption response
            con.phase = Phase::Encryption {
                client_address: *client_address,
                protocol_version: *protocol_version,
                server_address: server_address.clone(),
                server_port: *server_port,
                user_name: user_name.clone(),
                user_id: *user_id,
                verify_token,
                should_authenticate,
            };

            // create a new encryption request and send it
            let encryption_request = outbound::EncryptionRequestPacket {
                server_id: "".to_owned(),
                public_key: authentication::ENCODED_PUB.clone(),
                verify_token,
                should_authenticate,
            };
            debug!(
                packet = debug(&encryption_request),
                "sending encryption request packet"
            );
            con.write_packet(encryption_request).await?;

            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::tests::assert_packet;

    #[tokio::test]
    async fn packets() {
        assert_packet::<outbound::DisconnectPacket>(0x00).await;
        assert_packet::<outbound::EncryptionRequestPacket>(0x01).await;
        assert_packet::<outbound::LoginSuccessPacket>(0x02).await;
        assert_packet::<outbound::SetCompressionPacket>(0x03).await;
        assert_packet::<outbound::LoginPluginRequestPacket>(0x04).await;
        assert_packet::<outbound::CookieRequestPacket>(0x05).await;

        assert_packet::<inbound::LoginStartPacket>(0x00).await;
        assert_packet::<inbound::EncryptionResponsePacket>(0x01).await;
        assert_packet::<inbound::LoginPluginResponsePacket>(0x02).await;
        assert_packet::<inbound::LoginAcknowledgedPacket>(0x03).await;
        assert_packet::<inbound::CookieResponsePacket>(0x04).await;
    }
}
