pub mod k3s;

use crate::common::k3s::{K3s, KUBE_SECURE_PORT};
use kube::config::Kubeconfig;
use kube::{Client, Config};
use std::env::temp_dir;
use std::sync::{Arc, Weak};
use testcontainers::core::{AccessMode, CmdWaitFor, ExecCommand, Mount};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use tokio::sync::Mutex;

/// The local path to the Agones CRDs. This directory is mounted into the container such that we do
/// not have to download the CRDs every time.
const CRD_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/crds");

/// A static variable that holds a reference to the Agones container. This allows the container to
/// be dropped when no active test no longer references it.
static AGONES: Mutex<Option<Weak<AgonesSuite>>> = Mutex::const_new(None);

/// Gets a reference to the Agones container. If the container does not exist (or no longer exists),
/// it is created. The returned reference should be kept as long as possible to increase the chance
/// of the next test claiming another reference and preventing the instance from getting dropped.
pub async fn agones() -> Arc<AgonesSuite> {
    // Get the existing Agones container if it exists.
    let mut agones = AGONES.lock().await;
    if let Some(container) = agones.as_ref().and_then(Weak::upgrade) {
        return container;
    }

    // If no container exists, create a new one. This also locks the mutex such that no other test
    // may create a new container at the same time. The container is then stored in the static variable
    // as a weak reference. This allows the container to be dropped when no active test no longer references it.
    let container = Arc::new(AgonesSuite::start().await);
    *agones = Some(Arc::downgrade(&container));
    container
}

/// A [`AgonesSuite`] is a test suite that bundle a kubernetes cluster (containerized) with Agones and
/// a [`Client`] to interact with it. Use the [`AgonesSuite::start`] method to create a separate suite
/// and [`agones`] to get a shared one. In general, the shared suite should be preferred to save on
/// system resources and startup time.
pub struct AgonesSuite {
    #[allow(dead_code)]
    instance: ContainerAsync<K3s>,
    client: Client,
}

impl AsRef<Client> for AgonesSuite {
    fn as_ref(&self) -> &Client {
        &self.client
    }
}

impl AgonesSuite {
    /// Starts a new kubernetes cluster in a docker container and installs Agones into it. This may
    /// take up to two minutes. In general, the shared [`agones`] suite should be preferred to save on
    /// system resources and startup time.
    pub async fn start() -> Self {
        // Create the K3s container with the crds mounted. The instance also requires a temp directory
        // to place the kube config into such that it can be used to create a client.
        let crd_mount =
            Mount::bind_mount(CRD_PATH, "/etc/crds").with_access_mode(AccessMode::ReadOnly);
        let instance = K3s::default()
            .with_conf_mount(temp_dir())
            .with_mount(crd_mount)
            .with_privileged(true)
            .with_userns_mode("host")
            .start()
            .await
            .expect("Failed to start K3s container.");

        // Get the container configuration and build a kube client. This client is then used to interact
        // with the kubernetes cluster.
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
        let client = Client::try_from(config.clone()).expect("Failed to create Kubernetes client.");

        // Next, use the container kubectl binary to install the Agones CRDs. Following the agones
        // install documentation, we first create the agones namespace. This operation is immediately
        // applied.
        instance
            .exec(cmd(["kubectl", "create", "namespace", "agones-system"]))
            .await
            .expect("Failed to create Agones namespace");

        // Next, we apply the agones install CRD. The file is mounted into the container such that
        // we do not have to download it every time. We then wait for the CRDs to be installed.
        // By checking the status of the webhook, we can determine when the CRDs are ready. This
        // webhook is required for installing the gameserver.
        instance
            .exec(cmd([
                "kubectl",
                "apply",
                "--server-side",
                "-f",
                "/etc/crds/install.yaml",
            ]))
            .await
            .expect("Failed to install Agones CRDs");

        instance
            .exec(cmd([
                "kubectl",
                "wait",
                "endpoints",
                "agones-controller-service",
                "-n",
                "agones-system",
                "--for=jsonpath={.subsets[*].addresses[0].ip}",
                "--timeout=120s",
            ]))
            .await
            .expect("Failed to wait for webhook service endpoints");

        // Next, we create the gameserver from the mounted configuration file and wait for it to
        // complete.
        instance
            .exec(cmd([
                "kubectl",
                "create",
                "-f",
                "/etc/crds/gameserver.yaml",
            ]))
            .await
            .expect("Failed to create Agones gameserver");

        instance
            .exec(cmd([
                "kubectl",
                "wait",
                "--for=jsonpath={.status.state}=Ready",
                "--timeout=120s",
                "gameserver",
                "--all",
            ]))
            .await
            .expect("Failed to wait for gameserver to be ready");

        AgonesSuite { instance, client }
    }

    /// Gets a reference to the [`Client`] that can be used to interact with the kubernetes cluster.
    pub fn client(&self) -> &Client {
        &self.client
    }
}

/// Creates a new [`ExecCommand`] that waits until it completes successfully (exit code 0).
fn cmd(cmd: impl IntoIterator<Item = impl Into<String>>) -> ExecCommand {
    ExecCommand::new(cmd).with_cmd_ready_condition(CmdWaitFor::exit_code(0))
}
