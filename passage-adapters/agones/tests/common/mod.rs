pub mod k3s;

use crate::common::k3s::{K3s, KUBE_SECURE_PORT};
use kube::config::Kubeconfig;
use kube::{Client, Config};
use std::env::temp_dir;
use std::error::Error;
use testcontainers::core::{ExecCommand, ExecResult};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};

// TODO download file at compile time or add to sources
const AGONES_INSTALL_URL: &str = "https://raw.githubusercontent.com/googleforgames/agones/release-1.57.0/install/yaml/install.yaml";

const INSTALL_YAML: &str = include_str!("install.yaml");

const AGONES_EXAMPLE_GAMESERVER: &str = "https://raw.githubusercontent.com/googleforgames/agones/release-1.57.0/examples/simple-game-server/gameserver.yaml";

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
        let kube_port = instance
            .get_host_port_ipv4(KUBE_SECURE_PORT)
            .await
            .expect("Failed to read kube port.");
        let kube_conf_str = instance
            .image()
            .read_kube_config()
            .expect("Failed to read kubeconfig file.")
            .replace(
                &format!(":{}", KUBE_SECURE_PORT.as_u16()),
                &format!(":{kube_port}"),
            );
        let kube_conf = Kubeconfig::from_yaml(&kube_conf_str).expect("Failed to parse kube config");
        let config = Config::from_custom_kubeconfig(kube_conf, &Default::default())
            .await
            .expect("Failed to create Kubernetes client.");

        // Install all required CRDs.
        let client = Client::try_from(config.clone()).expect("Failed to create Kubernetes client.");

        instance
            .exec(ExecCommand::new([
                "kubectl",
                "create",
                "namespace",
                "agones-system",
            ]))
            .await
            .expect("Failed to create Agones namespace")
            .until_exit_code()
            .await
            .expect("Failed to create Agones namespace");

        instance
            .exec(ExecCommand::new([
                "kubectl",
                "apply",
                "--server-side",
                "-f",
                AGONES_INSTALL_URL,
            ]))
            .await
            .expect("Failed to install Agones CRDs")
            .until_exit_code()
            .await
            .expect("Failed to install Agones CRDs");

        // TODO maybe remove?
        instance
            .exec(ExecCommand::new([
                "kubectl",
                "wait",
                "--for=condition=established",
                "--timeout=120s",
                "crd/gameservers.agones.dev",
            ]))
            .await
            .expect("Failed to wait for gameserver to be ready")
            .until_exit_code()
            .await
            .expect("Failed to wait for gameserver to be ready");

        instance
            .exec(ExecCommand::new([
                "kubectl",
                "wait",
                "--for=condition=Available",
                "--timeout=120s",
                "deployment",
                "agones-controller",
                "-n",
                "agones-system",
            ]))
            .await
            .expect("Failed to wait for endpoints to be ready")
            .until_exit_code()
            .await
            .expect("Failed to wait for endpoints to be ready");

        // Create a gameserver to test with.
        static TRIES: usize = 10;
        for i in 0..TRIES {
            let ready = instance
                .exec(ExecCommand::new([
                    "kubectl",
                    "create",
                    "-f",
                    AGONES_EXAMPLE_GAMESERVER,
                ]))
                .await
                .expect("Failed to create Agones gameserver")
                .until_exit_code()
                .await;
            match ready {
                Ok(_) => break,
                Err(err) => {
                    if i == TRIES - 1 {
                        panic!("Failed to create Agones gameserver: {}", err);
                    }
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                }
            }
        }

        instance
            .exec(ExecCommand::new([
                "kubectl",
                "wait",
                "--for=jsonpath={.status.state}=Ready",
                "--timeout=120s",
                "gameserver",
                "--all",
            ]))
            .await
            .expect("Failed to wait for gameserver to be ready")
            .until_exit_code()
            .await
            .expect("Failed to wait for gameserver to be ready");

        K3sContainer { instance, client }
    }
}

pub trait ExecExt {
    fn until_exit_code(&mut self) -> impl Future<Output = Result<i64, Box<dyn std::error::Error>>>;
}

impl ExecExt for ExecResult {
    async fn until_exit_code(&mut self) -> Result<i64, Box<dyn Error>> {
        let stderr = self.stderr_to_vec().await?;
        if !stderr.is_empty() {
            return Err(String::from_utf8_lossy(&stderr).into());
        }
        Ok(self
            .exit_code()
            .await?
            .expect("Command completed without exit code."))
    }
}
