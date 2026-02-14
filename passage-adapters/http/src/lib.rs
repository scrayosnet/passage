use std::sync::LazyLock;

pub mod status_adapter;

// reexport adapters
pub use status_adapter::*;

static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .build()
        .expect("failed to build reqwest client")
});
