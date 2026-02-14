use crate::error::Result;
use crate::localization::LocalizationAdapter;
use std::collections::HashMap;
use tracing::{trace, warn};

#[derive(Debug)]
pub struct FixedLocalizationAdapter {
    default_locale: String,
    messages: HashMap<String, HashMap<String, String>>,
}

impl FixedLocalizationAdapter {
    pub fn new(default_locale: String, messages: HashMap<String, HashMap<String, String>>) -> Self {
        Self {
            default_locale,
            messages,
        }
    }
}

impl Default for FixedLocalizationAdapter {
    fn default() -> Self {
        Self {
            default_locale: "en_us".to_string(),
            messages: HashMap::new(),
        }
    }
}

impl LocalizationAdapter for FixedLocalizationAdapter {
    #[tracing::instrument(skip_all)]
    async fn localize(
        &self,
        locale: Option<&str>,
        key: &str,
        params: &[(&'static str, String)],
    ) -> Result<String> {
        trace!("localizing fixed");
        let locale = locale.unwrap_or(&self.default_locale);
        let locales = [
            locale,
            &locale[..2],
            &self.default_locale,
            &self.default_locale[..2],
        ];

        let mut locale_messages = None;
        for locale in &locales {
            locale_messages = self.messages.get(*locale);
            if locale_messages.is_some() {
                break;
            }
        }

        let Some(locale_messages) = locale_messages else {
            warn!(locales = ?locales, "cannot find locales");
            return Ok(key.to_string());
        };

        let Some(template) = locale_messages.get(key) else {
            return Ok(key.to_string());
        };

        let mut message = template.clone();
        for (param_key, param_val) in params {
            message = message.replace(param_key, param_val);
        }
        Ok(message)
    }
}
