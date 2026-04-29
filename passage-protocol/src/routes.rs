use passage_adapters::Adapters;
use regex::Regex;
use std::fmt::{Display, Formatter};
use std::sync::Arc;

pub type Routes<Stat, Disc, Auth, Loca> = Arc<[Route<Stat, Disc, Auth, Loca>]>;

pub struct Route<Stat, Disc, Auth, Loca> {
    pub hostname: Regex,
    pub name: Option<String>,
    pub adapters: Arc<Adapters<Stat, Disc, Auth, Loca>>,
}

impl<Stat, Disc, Auth, Loca> Display for Route<Stat, Disc, Auth, Loca> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Route({})",
            self.name.as_ref().unwrap_or(&self.hostname.to_string())
        )
    }
}
