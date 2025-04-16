use crate::adapter::resourcepack::{Resourcepack, ResourcepackSupplier};
use crate::protocol::Error;
use crate::status::Protocol;
use async_trait::async_trait;
use std::net::SocketAddr;
use uuid::{Uuid, uuid};

#[derive(Default)]
pub struct FixedResourcePackSupplier;

#[async_trait]
impl ResourcepackSupplier for FixedResourcePackSupplier {
    async fn get_resourcepacks(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
        _username: &str,
        _user_id: &Uuid,
    ) -> Result<Vec<Resourcepack>, Error> {
        Ok(vec![Resourcepack {
            uuid: uuid!("9c09eef4-f68d-4387-9751-72bbff53d5a0"),
            url: "https://impackable.justchunks.net/download/67e3e6e8704c701ec3cf5f8b".to_string(),
            hash: "c7affa49facf2b14238f1d2f7f04d7d0360bdb1d".to_string(),
            forced: true,
            prompt_message: Some("Please install!".to_string()),
        }])
    }
}
