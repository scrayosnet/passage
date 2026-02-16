use crate::config;
use passage_adapters::FixedLocalizationAdapter;
use passage_adapters::localization::LocalizationAdapter;
use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub enum DynLocalizationAdapter {
    Fixed(FixedLocalizationAdapter),
}

impl Display for DynLocalizationAdapter {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fixed(_) => write!(f, "fixed"),
        }
    }
}

impl LocalizationAdapter for DynLocalizationAdapter {
    async fn localize(
        &self,
        locale: Option<&str>,
        key: &str,
        params: &[(&'static str, String)],
    ) -> passage_adapters::Result<String> {
        match self {
            DynLocalizationAdapter::Fixed(adapter) => adapter.localize(locale, key, params).await,
        }
    }
}

impl DynLocalizationAdapter {
    pub async fn from_config(
        config: config::LocalizationAdapter,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        #[allow(unreachable_patterns)]
        match config {
            config::LocalizationAdapter::Fixed(config) => {
                let adapter = FixedLocalizationAdapter::new(config.default_locale, config.messages);
                Ok(DynLocalizationAdapter::Fixed(adapter))
            }
            _ => Err("unknown localization adapter configured".into()),
        }
    }
}
