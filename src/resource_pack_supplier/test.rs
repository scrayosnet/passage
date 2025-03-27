use crate::protocol::Error;
use crate::resource_pack_supplier::{ResourcePack, ResourcePackSupplier};
use crate::status::Protocol;
use async_trait::async_trait;
use std::net::SocketAddr;
use uuid::{Uuid, uuid};

#[derive(Default)]
pub struct TestResourcePackSupplier;

#[async_trait]
impl ResourcePackSupplier for TestResourcePackSupplier {
    async fn get_resource_packs(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
        _username: &str,
        _user_id: &Uuid,
    ) -> Result<Vec<ResourcePack>, Error> {
        Ok(vec![ResourcePack {
            uuid: uuid!("9c09eef4-f68d-4387-9751-72bbff53d5a0"),
            url: "https://impackable.justchunks.net/download/67e3e6e8704c701ec3cf5f8b".to_string(),
            hash: "c7affa49facf2b14238f1d2f7f04d7d0360bdb1d".to_string(),
            forced: true,
            prompt_message: "Please install!".to_string(),
        }])
    }
}
