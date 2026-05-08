use envtest::{Environment, Server};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::api::Api;
use kube::Client;

// TODO download file at compile time or add to sources
const AGONES_INSTALL_URL: &str = "https://raw.githubusercontent.com/googleforgames/agones/release-1.57.0/install/yaml/install.yaml";

const INSTALL_YAML: &str = include_str!("install.yaml");

const AGONES_EXAMPLE_GAMESERVER: &str = "https://raw.githubusercontent.com/googleforgames/agones/release-1.57.0/examples/simple-game-server/gameserver.yaml";

pub struct K3sContainer {
    pub server: Server,
    pub client: Client,
}

impl K3sContainer {
    pub async fn start() -> Self {
        // Create the envtest suite
        let mut env = Environment::default();
        env.crd_install_options.paths.push("/home/hydrofin/Projects/JustChunks/passage/passage-adapters/agones/tests/common/install.yaml".to_string());
            //.with_crds(INSTALL_YAML).expect("Failed to install agones CRD");
        let server = env.create().await.expect("Failed to create kube server");
        let client = server.client().expect("Failed to create kube client");

        // List all installed CRDs
        let crds: Api<CustomResourceDefinition> = Api::all(client.clone());
        let crd_list = crds.list(&Default::default()).await.expect("Failed to list CRDs");
        println!("Installed CRDs:");
        for crd in crd_list.items {
            println!("  - {}", crd.metadata.name.unwrap_or_default());
        }

        Self { server, client }
    }
}
