use passage_adapters::Client;
use passage_adapters::backoff::ExponentialBackoff;
use passage_adapters_agones::template::Template;
use passage_adapters_agones::{AgonesDiscoveryAdapter, AgonesDiscoveryAdapterConfig};

pub mod common;

#[tokio::test]
pub async fn allocate_match_label_selector() {
    // Create the kubernetes testcontainer with a client.
    let agones = common::agones().await;
    let kube_client = agones.client().clone();

    // Create the adapter instance.
    let config = AgonesDiscoveryAdapterConfig {
        namespace: Some("default".to_string()),
        backoff: ExponentialBackoff::once(),
        selectors: vec![Template::new(serde_json::json!({
            "matchLabels": { "game": "simple-game" }
        }))],
        ..Default::default()
    };
    let adapter = AgonesDiscoveryAdapter::new_with_client(kube_client, config)
        .await
        .expect("Failed to create adapter");

    // Allocate a server.
    let target = adapter
        .allocate(&Client::default())
        .await
        .expect("Failed to allocate server");
    assert!(target.is_some())
}

#[tokio::test]
pub async fn allocate_unmatch_label_selector() {
    // Create the kubernetes testcontainer with a client.
    let agones = common::agones().await;
    let kube_client = agones.client().clone();

    // Create the adapter instance.
    let config = AgonesDiscoveryAdapterConfig {
        namespace: Some("default".to_string()),
        backoff: ExponentialBackoff::once(),
        selectors: vec![Template::new(serde_json::json!({
            "matchLabels": { "game": "unknown-game" }
        }))],
        ..Default::default()
    };
    let adapter = AgonesDiscoveryAdapter::new_with_client(kube_client, config)
        .await
        .expect("Failed to create adapter");

    // Allocate a server.
    let target = adapter
        .allocate(&Client::default())
        .await
        .expect("Failed to allocate server");
    assert!(target.is_none())
}
