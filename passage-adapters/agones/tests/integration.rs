use common::AgonesContainer;
use passage_adapters::Client;
use passage_adapters_agones::template::Template;
use passage_adapters_agones::{AgonesDiscoveryAdapter, AgonesDiscoveryAdapterConfig};
use std::net::SocketAddr;

pub mod common;

#[tokio::test]
pub async fn test() {
    // Create the kubernetes testcontainer with a client.
    let kube_container = AgonesContainer::start().await;
    let kube_client = kube_container.client().clone();

    // Create the adapter instance.
    let config = AgonesDiscoveryAdapterConfig {
        namespace: Some("default".to_string()),
        selectors: vec![Template::new(serde_json::json!({
            "matchLabels": { "game": "simple-game" }
        }))],
        ..Default::default()
    };
    let adapter = AgonesDiscoveryAdapter::new_with_client(kube_client, config)
        .await
        .expect("Failed to create adapter");

    // Allocate a server.
    let client = Client {
        protocol_version: 0,
        server_address: "".to_string(),
        server_port: 0,
        address: SocketAddr::new("127.0.0.1".parse().unwrap(), 0),
    };
    let target = adapter
        .allocate(&client)
        .await
        .expect("Failed to allocate server");

    // TODO check that the server is allocated

    drop(kube_container);
    assert!(target.is_some())
}
