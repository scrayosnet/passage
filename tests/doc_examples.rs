//! Verifies that every Passage configuration example in the documentation actually deserializes
//! into [`Config`]. This keeps the documented YAML honest: a renamed field, a dropped adapter
//! variant or a typo in an example fails the build instead of silently misleading readers.
//!
//! Fenced `yaml` blocks are collected from `.docs/src/content/docs` and `config/example.yaml`.
//! Blocks that are not Passage configuration (Kubernetes manifests, OTel collector configs,
//! docker-compose files) are skipped, as are blocks containing the `{ ... }` placeholder used in
//! the structural overview.

use config::{File, FileFormat};
use passage::config::Config;
use regex::Regex;
use std::path::{Path, PathBuf};

/// Top-level keys that identify a block as a complete configuration.
const TOP_LEVEL_KEYS: &[&str] = &[
    "address",
    "timeout",
    "routes",
    "rate_limiter",
    "sentry",
    "otel",
    "proxy_protocol",
    "max_packet_length",
    "auth_cookie_expiry",
    "auth_secret",
    "system_observer_interval",
];

/// Keys that identify a block as a route-level fragment, which is wrapped into a route.
const ROUTE_KEYS: &[&str] = &[
    "hostname",
    "status",
    "authentication",
    "localization",
    "discovery",
];

struct Example {
    origin: String,
    yaml: String,
}

fn indent(block: &str, spaces: usize) -> String {
    let pad = " ".repeat(spaces);
    block
        .lines()
        .map(|line| {
            if line.trim().is_empty() {
                line.to_owned()
            } else {
                format!("{pad}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Wraps a documentation snippet into a complete configuration, or returns `None` if the block is
/// not Passage configuration at all.
fn as_config(block: &str) -> Option<String> {
    // The structural overview uses `{ ... }` placeholders and is illustrative only.
    if block.contains("{ ... }") {
        return None;
    }

    let key = Regex::new(r"(?m)^([a-z_]+):").unwrap();
    let keys: Vec<String> = key
        .captures_iter(block)
        .map(|c| c[1].to_owned())
        .collect::<Vec<_>>();

    if keys.iter().any(|k| TOP_LEVEL_KEYS.contains(&k.as_str())) {
        Some(block.to_owned())
    } else if keys.iter().any(|k| ROUTE_KEYS.contains(&k.as_str())) {
        Some(format!("routes:\n- hostname: \".*\"\n{}", indent(block, 2)))
    } else if Regex::new(r"^\s*-\s*type:").unwrap().is_match(block) {
        Some(format!(
            "routes:\n- hostname: \".*\"\n  discovery:\n    type: fixed_discovery\n    actions:\n{}",
            indent(block, 4)
        ))
    } else {
        None
    }
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("documentation directory is readable") {
        let path = entry.expect("readable directory entry").path();
        if path.is_dir() {
            collect(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("md") | Some("mdx")
        ) {
            out.push(path);
        }
    }
}

fn examples() -> Vec<Example> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    collect(&root.join(".docs/src/content/docs"), &mut files);
    files.sort();

    let fence = Regex::new(r"(?s)```ya?ml\n(.*?)```").unwrap();
    let mut examples = Vec::new();

    for file in files {
        let text = std::fs::read_to_string(&file).expect("documentation file is readable");
        let display = file
            .strip_prefix(&root)
            .unwrap_or(&file)
            .display()
            .to_string();
        for capture in fence.captures_iter(&text) {
            let block = capture.get(1).expect("fence body");
            let line = text[..block.start()].matches('\n').count() + 1;
            if let Some(yaml) = as_config(block.as_str()) {
                examples.push(Example {
                    origin: format!("{display}:{line}"),
                    yaml,
                });
            }
        }
    }

    let example_config = root.join("config/example.yaml");
    examples.push(Example {
        origin: "config/example.yaml".to_owned(),
        yaml: std::fs::read_to_string(example_config).expect("example config is readable"),
    });

    examples
}

#[test]
fn documented_yaml_examples_deserialize() {
    let examples = examples();
    assert!(
        examples.len() > 50,
        "expected to find the documentation examples, found {}",
        examples.len()
    );

    let mut failures = Vec::new();
    for example in &examples {
        let result = config::Config::builder()
            .add_source(File::from_str(&example.yaml, FileFormat::Yaml))
            .build()
            .and_then(config::Config::try_deserialize::<Config>);
        if let Err(err) = result {
            failures.push(format!("{}: {err}", example.origin));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} documentation examples do not deserialize:\n{}",
        failures.len(),
        examples.len(),
        failures.join("\n")
    );
}
