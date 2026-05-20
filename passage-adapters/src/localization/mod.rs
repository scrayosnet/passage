pub mod fixed;

use crate::error::Result;
use std::fmt::Debug;

/// A [`LocalizationAdapter`] is used to localize messages based on a message key, locale, and template
/// params. The `key` identifies a message template. The `locale` is the Mojang locale tag reported
/// by the client (e.g. `"en_US"`); implementations should fall back to a default locale when the
/// requested locale has no entry. The `params` are named substitution values applied to the template.
///
/// If the key is not found, implementations should return the key itself rather than an error.
pub trait LocalizationAdapter: Debug + Send + Sync {
    fn localize(
        &self,
        locale: Option<&str>,
        key: &str,
        params: &[(&'static str, String)],
    ) -> impl Future<Output = Result<String>> + Send;
}
