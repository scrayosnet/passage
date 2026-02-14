use serde::Deserialize;
use std::collections::HashMap;
use tracing::warn;

// TODO convert into an adapter? Or move into module first
/// [Localization] holds all localizable messages of the application.
#[derive(Debug, Clone, Deserialize)]
pub struct Localization {
    /// The locale to be used in case the client locale is unknown or unsupported.
    #[serde(alias = "defaultlocale")]
    pub default_locale: String,

    /// The localizable messages.
    pub messages: HashMap<String, HashMap<String, String>>,
}

impl Default for Localization {
    fn default() -> Self {
        Self {
            default_locale: "en_US".to_string(),
            messages: HashMap::new(),
        }
    }
}

impl Localization {
    #[must_use]
    pub fn localize_default(&self, key: &str, params: &[(&'static str, String)]) -> String {
        self.localize(&self.default_locale, key, params)
    }

    #[must_use]
    pub fn localize(&self, locale: &str, key: &str, params: &[(&'static str, String)]) -> String {
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
            return key.to_string();
        };

        let Some(template) = locale_messages.get(key) else {
            return key.to_string();
        };

        let mut message = template.clone();
        for (param_key, param_val) in params {
            message = message.replace(param_key, param_val);
        }
        message
    }
}
