use crate::adapter::resourcepack::{Resourcepack, ResourcepackSupplier};
use crate::connection::Error;
use crate::status::Protocol;
use async_trait::async_trait;
use uuid::Uuid;

#[derive(Default)]
pub struct NoneResourcePackSupplier;

#[async_trait]
impl ResourcepackSupplier for NoneResourcePackSupplier {
    async fn get_resourcepacks(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
        _username: &str,
        _user_id: &Uuid,
    ) -> Result<Vec<Resourcepack>, Error> {
        Ok(vec![])
    }
}
