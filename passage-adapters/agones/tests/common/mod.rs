use futures_util::StreamExt;
use k8s_openapi::api::core::v1::Namespace;
use kube::api::PostParams;
use kube::config::Kubeconfig;
use kube::{Api, Client, Config};
use std::env::temp_dir;
use testcontainers::core::ExecCommand;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::k3s::{K3s, KUBE_SECURE_PORT};

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
            .replace(&format!(":{}", KUBE_SECURE_PORT.as_u16()), &format!(":{kube_port}"));
        let kube_conf = Kubeconfig::from_yaml(&kube_conf_str)
            .expect("Failed to parse kube config");
        let config = Config::from_custom_kubeconfig(kube_conf, &Default::default())
            .await
            .expect("Failed to create Kubernetes client.");

        // Install all required CRDs.
        let client = Client::try_from(config.clone()).expect("Failed to create Kubernetes client.");
        instance
            .exec(ExecCommand::new(["kubectl", "create", "namespace", "agones-system"]))
            .await
            .expect("Failed to install Agones CRDs");
        instance
            .exec(ExecCommand::new([
                "kubectl", "wait", "--for=jsonpath={.status.phase}=Active",
                "--timeout=120s", "namespace/agones-system"
            ]))
            .await
            .expect("Failed to wait for gameserver to be ready");
        println!("Agones namespace created.");

        instance
            .exec(ExecCommand::new(["kubectl", "apply", "--server-side", "-f", AGONES_INSTALL_URL]))
            .await
            .expect("Failed to install Agones CRDs");
        instance
            .exec(ExecCommand::new([
                "kubectl", "wait", "--for=condition=available",
                "--timeout=120s", "deployment/agones-controller",
                "-n", "agones-system",
            ]))
            .await
            .expect("Failed to wait for gameserver to be ready");
        instance
            .exec(ExecCommand::new([
                "kubectl", "wait", "--for=condition=established",
                "--timeout=120s", "crd/gameservers.agones.dev"
            ]))
            .await
            .expect("Failed to wait for gameserver to be ready");
        instance
            .exec(ExecCommand::new([
                "kubectl", "wait", "--for=condition=established",
                "--timeout=120s", "crd/fleets.agones.dev"
            ]))
            .await
            .expect("Failed to wait for gameserver to be ready");
        instance
            .exec(ExecCommand::new([
                "kubectl", "wait", "--for=condition=established",
                "--timeout=120s", "crd/gameserversets.agones.dev"
            ]))
            .await
            .expect("Failed to wait for gameserver to be ready");
        instance
            .exec(ExecCommand::new([
                "kubectl", "wait", "--for=jsonpath='{.subsets[*].addresses[0].ip}'",
                "endpoints/agones-controller-service",
                "-n", "agones-system",
            ]))
            .await
            .expect("Failed to wait for gameserver to be ready");

        // Create a gameserver to test with.
        let status = instance
            .exec(ExecCommand::new(["kubectl", "create", "-f", AGONES_EXAMPLE_GAMESERVER]))
            .await
            .expect("Failed to create Agones gameserver")
            .exit_code()
            .await
            .expect("Failed to create Agones gameserver");
        println!("status {:?}", status);
        instance
            .exec(ExecCommand::new([
                "kubectl", "wait", "--for=jsonpath={.status.state}=Ready",
                "--timeout=120s", "gameserver", "--all"
            ]))
            .await
            .expect("Failed to wait for gameserver to be ready");

        println!("Agones example gameserver created.");

        tokio::time::sleep(std::time::Duration::from_hours(10)).await;

        K3sContainer { instance, client }
    }
}

async fn create_namespace(client: Client, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let api: Api<Namespace> = Api::all(client.clone());
    let namespace = Namespace {
        metadata: kube::api::ObjectMeta {
            name: Some(name.to_string()),
            ..Default::default()
        },
        ..Default::default()
    };
    match api.create(&PostParams::default(), &namespace).await {
        Ok(_) => {
            println!("Namespace '{}' creation initiated.", name);
            Ok(())
        },
        Err(kube::Error::Api(e)) if e.code == 409 => {
            println!("Namespace '{}' already exists, proceeding to wait for readiness.", name);
        }
        Err(e) => Err(e.into()),
    }
}
