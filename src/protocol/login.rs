use crate::authentication::VerifyToken;
use crate::protocol::{
    AsyncReadPacket, AsyncWritePacket, Error, InboundPacket, OutboundPacket, Packet,
};
use std::io::Cursor;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

#[derive(Debug)]
pub struct LoginStartPacket {
    pub name: String,
    pub user_id: Uuid,
}

impl Packet for LoginStartPacket {
    fn get_packet_id() -> usize {
        0x00
    }
}

impl InboundPacket for LoginStartPacket {
    async fn new_from_buffer(buffer: &[u8]) -> Result<Self, Error> {
        let mut reader = Cursor::new(buffer);

        let name = reader.read_string().await?;
        let user_id = reader.read_uuid().await?;

        Ok(Self { name, user_id })
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
    async fn new_from_buffer(buffer: &[u8]) -> Result<Self, Error> {
        let mut reader = Cursor::new(buffer);

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
    async fn new_from_buffer(_buffer: &[u8]) -> Result<Self, Error> {
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
    async fn to_buffer(&self) -> Result<Vec<u8>, Error> {
        let mut buffer = Cursor::new(Vec::<u8>::new());

        buffer.write_string("").await?;
        buffer.write_bytes(&self.public_key).await?;
        buffer.write_bytes(&self.verify_token).await?;
        buffer.write_u8(self.should_authenticate as u8).await?;

        Ok(buffer.into_inner())
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
    async fn to_buffer(&self) -> Result<Vec<u8>, Error> {
        let mut buffer = Cursor::new(Vec::<u8>::new());

        buffer.write_uuid(&self.user_id).await?;
        buffer.write_string(&self.user_name).await?;
        // no properties in array
        buffer.write_varint(0).await?;

        Ok(buffer.into_inner())
    }
}
