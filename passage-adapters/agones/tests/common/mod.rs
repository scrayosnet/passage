use kube::config::Kubeconfig;
use kube::{Client, Config};
use std::env::temp_dir;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::k3s::K3s;

pub struct K3sContainer {
    pub instance: ContainerAsync<K3s>,
    pub client: Client,
}

impl K3sContainer {
    pub async fn start() -> Self {
        // Create the K3s container.
        let instance = K3s::default()
            .with_conf_mount(temp_dir())
            .with_privileged(true)
            .with_userns_mode("host")
            .start()
            .await
            .expect("Failed to start K3s container.");

        // Get the container configuration.
        let kube_conf_str = instance
            .image()
            .read_kube_config()
            .expect("Failed to read kubeconfig file.");
        let kube_conf = Kubeconfig::from_yaml(&kube_conf_str).unwrap();
        let config = Config::from_custom_kubeconfig(kube_conf, &Default::default())
            .await
            .expect("Failed to create Kubernetes client.");

        // Install all required CRDs.
        let client = Client::try_from(config.clone()).expect("Failed to create Kubernetes client.");
        // TODO install agones-sdk

        K3sContainer { instance, client }
    }
}
