pub mod fixed;

use crate::error::Result;
use std::fmt::Debug;

pub trait LocalizationAdapter: Debug + Send + Sync {
    fn localize(
        &self,
        locale: Option<&str>,
        key: &str,
        params: &[(&'static str, String)],
    ) -> impl Future<Output = Result<String>> + Send;
}
