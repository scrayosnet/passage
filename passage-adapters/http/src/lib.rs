use std::sync::LazyLock;

pub mod mojang_adapter;
pub mod status_adapter;

// reexport adapters
pub use mojang_adapter::MojangAdapter;
pub use status_adapter::HttpStatusAdapter;

static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .build()
        .expect("failed to build reqwest client")
});
