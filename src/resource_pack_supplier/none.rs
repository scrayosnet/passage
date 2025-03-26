use crate::protocol::Error;
use crate::resource_pack_supplier::{ResourcePack, ResourcePackSupplier};
use crate::status::Protocol;
use std::net::SocketAddr;
use uuid::Uuid;

#[derive(Default)]
pub struct NoneResourcePackSupplier;

impl ResourcePackSupplier for NoneResourcePackSupplier {
    async fn get_resource_packs(
        &self,
        client_addr: &SocketAddr,
        server_addr: &(String, u16),
        protocol: Protocol,
        username: &str,
        user_id: &Uuid,
    ) -> Result<Vec<ResourcePack>, Error> {
        Ok(Vec::new())
    }
}
