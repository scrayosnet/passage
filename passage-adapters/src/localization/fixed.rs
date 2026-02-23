use crate::error::Result;
use crate::localization::LocalizationAdapter;
use std::collections::HashMap;
use tracing::{debug, trace, warn};

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

    /// Splits a locale string into its language and country parts (splitting by `_`) with decreasing
    /// precision and adds them to the given vector.
    ///
    /// For example, the locale `en_US` is converted into the vector elements `["en_US", "en"]`.
    fn append_locale<'a>(&self, locale: &'a str, locales: &mut Vec<&'a str>) {
        // get all occurrences of '_' in the locale string
        let indices = locale
            .match_indices('_')
            .map(|x| x.0)
            .collect::<Vec<usize>>();

        // build decreasing slices
        locales.push(locale);
        for i in indices.iter().rev() {
            locales.push(&locale[..*i]);
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
        // get locales to check in order (e.g., 'de_DE' -> 'de', -> 'en_US' -> 'en')
        let locale = locale.unwrap_or(&self.default_locale);
        let mut locales = vec![];
        self.append_locale(locale, &mut locales);
        self.append_locale(&self.default_locale, &mut locales);
        debug!(locales = ?locales, "build locales");

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
