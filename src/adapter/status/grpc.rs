use crate::adapter::status::StatusSupplier;
use crate::connection::Error;
use crate::status::{Protocol, ServerStatus};
use async_trait::async_trait;

#[derive(Default)]
pub struct HiddenStatusSupplier;

#[async_trait]
impl StatusSupplier for HiddenStatusSupplier {
    async fn get_status(
        &self,
        _client_addr: &SocketAddr,
        _server_addr: (&str, u16),
        _protocol: Protocol,
    ) -> Result<Option<ServerStatus>, Error> {
        Ok(None)
    }
}
