//! This crate provides HTTP-based adapters.
//!
//! It contains the [`MojangAdapter`] for official Minecraft authentication and the
//! [`HttpStatusAdapter`] for polling a remote HTTP endpoint for server status.

use std::sync::LazyLock;

pub mod mojang_adapter;
pub mod status_adapter;

// reexport adapters
pub use mojang_adapter::MojangAdapter;
pub use status_adapter::HttpStatusAdapter;

/// The shared HTTP client for all adapters.
static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .build()
        .expect("failed to build reqwest client")
});
