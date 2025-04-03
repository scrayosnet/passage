use crate::authentication;
use crate::authentication::VerifyToken;
use crate::connection::{Connection, Login};
use crate::protocol::{
    AsyncReadPacket, AsyncWritePacket, Error, InboundPacket, OutboundPacket, Packet, Phase,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::{debug, info};
use uuid::Uuid;

/// The outbound [`DisconnectPacket`]. (Placeholder)
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Disconnect_(login))
#[derive(Debug)]
pub struct DisconnectPacket {
    /// The JSON text component containing the reason of the disconnect.
    reason: String,
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

/// The outbound [`EncryptionRequestPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Encryption_Request)
#[derive(Debug)]
pub struct EncryptionRequestPacket {
    // server id - is always empty, so we skip it
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
        buffer.write_string("").await?;
        buffer.write_bytes(&self.public_key).await?;
        buffer.write_bytes(&self.verify_token).await?;
        buffer.write_bool(self.should_authenticate).await?;

        Ok(())
    }
}

/// The outbound [`LoginSuccessPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Login_Success)
#[derive(Debug)]
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

/// The outbound [`SetCompressionPacket`]. (Placeholder)
///
/// Enables compression. If compression is enabled, all following packets are encoded in the compressed
/// packet format. Negative values will disable compression, meaning the packet format should remain
/// in the uncompressed packet format. However, this packet is entirely optional, and if not sent,
/// compression will also not be enabled (the vanilla server does not send the packet when compression
/// is disabled).
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Set_Compression)
#[derive(Debug)]
#[deprecated(note = "placeholder implementation")]
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

/// The outbound [`LoginPluginRequestPacket`]. (Placeholder)
///
/// Used to implement a custom handshaking flow together with Login Plugin Response. Unlike plugin
/// messages in "play" mode, these messages follow a lock-step request/response scheme, where the
/// client is expected to respond to a request indicating whether it understood. The vanilla client
/// always responds that it hasn't understood, and sends an empty payload.
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Login_Plugin_Request)
#[derive(Debug)]
#[deprecated(note = "placeholder implementation")]
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

/// The outbound [`CookieRequestPacket`]. (Placeholder)
///
/// Requests a cookie that was previously stored.
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Cookie_Request_(login))
#[derive(Debug)]
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

/// The inbound [`LoginStartPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Login_Start)
#[derive(Debug)]
pub struct LoginStartPacket {
    pub user_name: String,
    pub user_id: Uuid,
}

impl Packet for LoginStartPacket {
    fn get_packet_id() -> usize {
        0x00
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

        // encode public key and generate verify token
        let verify_token = authentication::generate_token()?;

        // save packet information to connection
        con.login = Some(Login {
            verify_token: verify_token.clone(),
            user_name: self.user_name.clone(),
            user_id: self.user_id.clone(),
            success: false,
        });

        // create a new encryption request and send it
        let encryption_request = EncryptionRequestPacket {
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
#[derive(Debug)]
pub struct EncryptionResponsePacket {
    pub shared_secret: Vec<u8>,
    pub verify_token: Vec<u8>,
}

impl Packet for EncryptionResponsePacket {
    fn get_packet_id() -> usize {
        0x01
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

        // get login state from connection
        let Some(login) = &mut con.login else {
            return Err(Error::Generic("invalid state".to_string()));
        };

        // decrypt the shared secret and verify token
        let shared_secret =
            authentication::decrypt(&authentication::KEY_PAIR.0, &self.shared_secret)?;
        let decrypted_verify_token =
            authentication::decrypt(&authentication::KEY_PAIR.0, &self.verify_token)?;

        // verify the token is correct
        authentication::verify_token(login.verify_token.clone(), &decrypted_verify_token)?;

        // get the data for login success
        let auth_response = authentication::authenticate_mojang(
            &login.user_name,
            &shared_secret,
            &authentication::ENCODED_PUB,
        )
        .await?;

        // set success state and override login state
        login.success = true;
        login.user_id = auth_response.id.clone();
        login.user_name = auth_response.name.clone();

        // enable encryption for the connection using the shared secret
        con.enable_encryption(&shared_secret)?;

        // create a new login success packet and send it
        let login_success = LoginSuccessPacket {
            user_id: auth_response.id,
            user_name: auth_response.name,
        };
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
#[derive(Debug)]
#[deprecated(note = "placeholder implementation")]
pub struct LoginPluginResponsePacket;

impl Packet for LoginPluginResponsePacket {
    fn get_packet_id() -> usize {
        0x02
    }
}

impl InboundPacket for LoginPluginResponsePacket {
    async fn new_from_buffer<S>(_buffer: &mut S) -> Result<Self, Error>
    where
        S: AsyncRead + Unpin + Send + Sync,
    {
        Ok(Self)
    }

    async fn handle<S>(self, _con: &mut Connection<S>) -> Result<(), Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
    {
        Ok(())
    }
}

/// The inbound [`LoginAcknowledgedPacket`].
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Login_Acknowledged)
#[derive(Debug)]
pub struct LoginAcknowledgedPacket;

impl Packet for LoginAcknowledgedPacket {
    fn get_packet_id() -> usize {
        0x03
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

        // switch to configuration phase
        info!("switching to configuration phase");
        con.phase = Phase::Configuration;

        // handle configuration
        con.configure().await
    }
}

/// The inbound [`CookieResponsePacket`]. (Placeholder)
///
/// [Minecraft Docs](https://minecraft.wiki/w/Java_Edition_protocol#Cookie_Response_(login))
#[derive(Debug)]
pub struct CookieResponsePacket {
    pub key: String,
    pub payload: Option<Vec<u8>>,
}

impl Packet for CookieResponsePacket {
    fn get_packet_id() -> usize {
        0x04
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

    async fn handle<S>(self, _con: &mut Connection<S>) -> Result<(), Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + Sync,
    {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngCore;
    use std::io::Cursor;
    use tokio::io::AsyncReadExt;
    use uuid::uuid;

    #[tokio::test]
    async fn packet_ids_valid() {
        assert_eq!(LoginStartPacket::get_packet_id(), 0x00);
        assert_eq!(EncryptionResponsePacket::get_packet_id(), 0x01);
        assert_eq!(LoginAcknowledgedPacket::get_packet_id(), 0x03);
        assert_eq!(EncryptionRequestPacket::get_packet_id(), 0x01);
        assert_eq!(LoginSuccessPacket::get_packet_id(), 0x02);
    }

    #[tokio::test]
    async fn decode_login_start() {
        let user_name = "Scrayos";
        let user_id = uuid!("9c09eef4-f68d-4387-9751-72bbff53d5a0");

        let mut buffer: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        buffer.write_string(user_name).await.unwrap();
        buffer.write_uuid(&user_id).await.unwrap();

        let mut read_buffer: Cursor<Vec<u8>> = Cursor::new(buffer.into_inner());
        let packet = LoginStartPacket::new_from_buffer(&mut read_buffer)
            .await
            .unwrap();
        assert_eq!(packet.user_name, user_name);
        assert_eq!(packet.user_id, user_id);

        assert_eq!(
            read_buffer.position() as usize,
            read_buffer.get_ref().len(),
            "There are remaining bytes in the buffer"
        );
    }

    #[tokio::test]
    async fn decode_encryption_response() {
        let mut rng = rand::thread_rng();
        let mut shared_secret = [0u8; 32];
        rng.try_fill_bytes(&mut shared_secret).unwrap();
        let mut verify_token = [0u8; 32];
        rng.try_fill_bytes(&mut verify_token).unwrap();

        let mut buffer: Cursor<Vec<u8>> = Cursor::new(Vec::new());
        buffer.write_bytes(&shared_secret).await.unwrap();
        buffer.write_bytes(&verify_token).await.unwrap();

        let mut read_buffer: Cursor<Vec<u8>> = Cursor::new(buffer.into_inner());
        let packet = EncryptionResponsePacket::new_from_buffer(&mut read_buffer)
            .await
            .unwrap();
        assert_eq!(packet.shared_secret, shared_secret);
        assert_eq!(packet.verify_token, verify_token);

        assert_eq!(
            read_buffer.position() as usize,
            read_buffer.get_ref().len(),
            "There are remaining bytes in the buffer"
        );
    }

    #[tokio::test]
    async fn decode_login_acknowledged() {
        let mut buffer: Cursor<Vec<u8>> = Cursor::new(Vec::new());

        let _packet = LoginAcknowledgedPacket::new_from_buffer(&mut buffer)
            .await
            .unwrap();
        assert_eq!(
            buffer.position() as usize,
            buffer.get_ref().len(),
            "There are remaining bytes in the buffer"
        );
    }

    #[tokio::test]
    async fn encode_encryption_request() {
        let mut rng = rand::thread_rng();
        let mut public_key_write = [0u8; 32];
        rng.try_fill_bytes(&mut public_key_write).unwrap();
        let mut verify_token_write = [0u8; 32];
        rng.try_fill_bytes(&mut verify_token_write).unwrap();

        // write the packet into a buffer and box it as a slice (sized)
        let packet = EncryptionRequestPacket {
            public_key: public_key_write.to_vec(),
            verify_token: verify_token_write,
            should_authenticate: true,
        };
        let mut packet_buffer = Cursor::new(Vec::<u8>::new());
        packet.write_to_buffer(&mut packet_buffer).await.unwrap();
        let mut buffer: Cursor<Vec<u8>> = Cursor::new(packet_buffer.into_inner());

        let server_id = buffer.read_string().await.unwrap();
        let public_key = buffer.read_bytes().await.unwrap();
        let verify_token = buffer.read_bytes().await.unwrap();
        let should_authenticate = buffer.read_u8().await.unwrap();
        assert_eq!(server_id, "");
        assert_eq!(public_key, packet.public_key);
        assert_eq!(verify_token, packet.verify_token);
        assert_eq!(should_authenticate != 0, packet.should_authenticate);

        assert_eq!(
            buffer.position() as usize,
            buffer.get_ref().len(),
            "There are remaining bytes in the buffer"
        );
    }

    #[tokio::test]
    async fn encode_login_success() {
        // write the packet into a buffer and box it as a slice (sized)
        let packet = LoginSuccessPacket {
            user_id: uuid!("9c09eef4-f68d-4387-9751-72bbff53d5a0"),
            user_name: "Scrayos".to_string(),
        };
        let mut packet_buffer = Cursor::new(Vec::<u8>::new());
        packet.write_to_buffer(&mut packet_buffer).await.unwrap();
        let mut buffer: Cursor<Vec<u8>> = Cursor::new(packet_buffer.into_inner());

        let user_id = buffer.read_uuid().await.unwrap();
        let user_name = buffer.read_string().await.unwrap();
        let property_count = buffer.read_varint().await.unwrap();
        assert_eq!(user_id, packet.user_id);
        assert_eq!(user_name, packet.user_name);
        assert_eq!(property_count, 0);

        assert_eq!(
            buffer.position() as usize,
            buffer.get_ref().len(),
            "There are remaining bytes in the buffer"
        );
    }
}
