use crate::authentication::VerifyToken;
use crate::protocol::{
    AsyncReadPacket, AsyncWritePacket, Error, InboundPacket, OutboundPacket, Packet,
};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

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
}

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
}

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
}

#[derive(Debug)]
pub struct EncryptionRequestPacket {
    // server id - is always empty, so we skip it
    public_key: Vec<u8>,
    verify_token: VerifyToken,
    should_authenticate: bool,
}

impl EncryptionRequestPacket {
    pub const fn new(
        public_key: Vec<u8>,
        verify_token: VerifyToken,
        should_authenticate: bool,
    ) -> Self {
        Self {
            public_key,
            verify_token,
            should_authenticate,
        }
    }
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
        buffer.write_u8(self.should_authenticate as u8).await?;

        Ok(())
    }
}

#[derive(Debug)]
pub struct LoginSuccessPacket {
    user_id: Uuid,
    user_name: String,
    // properties - we don't need those
}

impl LoginSuccessPacket {
    pub const fn new(user_id: Uuid, user_name: String) -> Self {
        Self { user_id, user_name }
    }
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
        let packet =
            EncryptionRequestPacket::new(public_key_write.to_vec(), verify_token_write, true);
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
        let packet = LoginSuccessPacket::new(
            uuid!("9c09eef4-f68d-4387-9751-72bbff53d5a0"),
            "Scrayos".to_string(),
        );
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
