use crate::config;
use passage_adapters::FixedLocalizationAdapter;
use passage_adapters::localization::LocalizationAdapter;

#[derive(Debug)]
pub enum DynLocalizationAdapter {
    Fixed(FixedLocalizationAdapter),
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
        config: &config::TargetStrategy,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        match config.adapter.as_str() {
            "fixed" => {
                let Some(config) = config.fixed.clone() else {
                    return Err("fixed strategy adapter requires a configuration".into());
                };
                // TODO get profile from config
                let adapter = FixedLocalizationAdapter::new();
                Ok(DynLocalizationAdapter::Fixed(adapter))
            }
            _ => Err("unknown localization adapter configured".into()),
        }
    }
}
