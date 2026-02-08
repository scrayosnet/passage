---
title: Target Discovery Adapters
description: Configure how Passage discovers available backend servers.
---

Discovery adapters provide the list of available backend servers with metadata (identifier, address, type, player count, region, etc.).

## Fixed Adapter

Static server list from configuration.

```toml
[target_discovery]
adapter = "fixed"

[[target_discovery.fixed.targets]]
identifier = "hub-1"
address = "10.0.1.10:25565"
meta = { type = "hub", players = "0" }

[[target_discovery.fixed.targets]]
identifier = "survival-1"
address = "10.0.2.10:25565"
meta = { type = "survival", players = "0" }
```

**Metadata:** Used by strategy adapters for selection. Common fields: `type`, `region`, `players`, `capacity`.

## gRPC Adapter

Custom gRPC service for dynamic discovery.

```toml
[target_discovery]
adapter = "grpc"

[target_discovery.grpc]
address = "http://discovery-service:3030"
```

See [Custom gRPC Adapters](/advanced/custom-grpc-adapters/) for implementation details and proto definitions.

## Agones Adapter

Auto-discovers game servers in a Kubernetes cluster using Agones.

```toml
[target_discovery]
adapter = "agones"

[target_discovery.agones]
namespace = "minecraft"
label_selector = "game=minecraft,type=lobby"  # Optional
```

Watches for Agones `GameServer` resources in the namespace, extracts metadata from labels, annotations, and counters.

**RBAC:** Requires `get`, `list`, `watch` permissions on `gameservers` resource. See [Kubernetes Guide](/setup/kubernetes/).
