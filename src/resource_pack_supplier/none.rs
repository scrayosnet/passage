use crate::protocol::Error;
use crate::resource_pack_supplier::{ResourcePack, ResourcePackSupplier};
use crate::status::Protocol;
use async_trait::async_trait;
use std::net::SocketAddr;
use uuid::Uuid;

#[derive(Default)]
pub struct NoneResourcePackSupplier;

#[async_trait]
impl ResourcePackSupplier for NoneResourcePackSupplier {
    fn get_resource_packs(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
        _username: &str,
        _user_id: &Uuid,
    ) -> Result<Vec<ResourcePack>, Error> {
        Ok(vec![])
    }
}
