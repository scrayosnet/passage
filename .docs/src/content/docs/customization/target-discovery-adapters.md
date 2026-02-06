---
title: Target Discovery Adapters
description: Configure how Passage discovers available backend servers.
---

Target Discovery adapters determine which backend Minecraft servers are available to route players to. They provide a list of targets with metadata that can be used by the Target Strategy adapter for selection.

## Overview

During each player connection, Passage queries the Target Discovery adapter to get the current list of available servers. Each target includes:
- **Identifier**: Unique name for the server
- **Address**: IP and port to transfer the player to
- **Metadata**: Key-value pairs with server information (type, player count, region, etc.)

## Available Adapters

### Fixed Discovery Adapter

Returns a static list of servers from configuration.

**Use when:** You have a fixed set of backend servers that don't change dynamically.

#### Configuration

```toml
[target_discovery]
adapter = "fixed"

[[target_discovery.fixed.targets]]
identifier = "hub-1"
address = "10.0.1.10:25565"
meta = { type = "hub", region = "us-east", players = "0" }

[[target_discovery.fixed.targets]]
identifier = "hub-2"
address = "10.0.1.11:25565"
meta = { type = "hub", region = "us-west", players = "0" }

[[target_discovery.fixed.targets]]
identifier = "survival-1"
address = "10.0.2.10:25565"
meta = { type = "survival", difficulty = "hard" }
```

#### Parameters

Each target requires:
- **`identifier`** (string, required): Unique server identifier
- **`address`** (string, required): Server address in format `ip:port`
- **`meta`** (table, optional): Key-value pairs with server metadata

#### Metadata Usage

Metadata is used by the Target Strategy adapter for selection logic. Common metadata fields:

- `type`: Server type (hub, lobby, survival, creative, minigame)
- `region`: Geographic region (us-east, eu-west, asia)
- `players`: Current player count (updated manually or via external script)
- `capacity`: Maximum players
- `version`: Minecraft version (1.20, 1.21, etc.)
- `mode`: Game mode (easy, normal, hard, pvp, etc.)

**Note:** With the fixed adapter, metadata is static. Use gRPC or Agones adapters for dynamic metadata.

#### Example: Multi-Region Setup

```toml
[target_discovery]
adapter = "fixed"

# US East Servers
[[target_discovery.fixed.targets]]
identifier = "us-east-hub-1"
address = "10.10.1.10:25565"
meta = { type = "hub", region = "us-east" }

[[target_discovery.fixed.targets]]
identifier = "us-east-survival-1"
address = "10.10.1.20:25565"
meta = { type = "survival", region = "us-east", difficulty = "normal" }

# EU West Servers
[[target_discovery.fixed.targets]]
identifier = "eu-west-hub-1"
address = "10.20.1.10:25565"
meta = { type = "hub", region = "eu-west" }

[[target_discovery.fixed.targets]]
identifier = "eu-west-survival-1"
address = "10.20.1.20:25565"
meta = { type = "survival", region = "eu-west", difficulty = "hard" }
```

---

### gRPC Discovery Adapter

Queries a gRPC service for the current list of servers.

**Use when:** You have dynamic server management and want full control over discovery logic.

#### Configuration

```toml
[target_discovery]
adapter = "grpc"

[target_discovery.grpc]
address = "http://discovery-service:3030"
```

#### Parameters

- **`address`** (string, required): Address of the gRPC discovery service

#### gRPC Service Definition

Your service must implement the `Discovery` service:

```protobuf
syntax = "proto3";

service Discovery {
    rpc GetTargets(TargetRequest) returns (TargetsResponse);
}

message TargetRequest {
    Address client_address = 1;
    Address server_address = 2;
    uint64 protocol = 3;
    string username = 4;
    string user_id = 5;
}

message TargetsResponse {
    repeated Target targets = 1;
}

message Target {
    string identifier = 1;
    Address address = 2;
    repeated MetaEntry meta = 3;
}

message MetaEntry {
    string key = 1;
    string value = 2;
}
```

#### Request Context

Your service receives:
- **client_address**: IP and port of connecting player
- **server_address**: Domain/IP the player connected to
- **protocol**: Minecraft protocol version
- **username**: Player's username (e.g., "Steve")
- **user_id**: Player's UUID

Use this context to return appropriate servers (e.g., region-specific, version-specific, or player-specific).

#### Example Implementation (Go)

```go
package main

import (
    "context"
    "database/sql"
    pb "github.com/scrayosnet/passage/proto/adapter"
}

type discoveryServer struct {
    db *sql.DB
    pb.UnimplementedDiscoveryServer
}

func (s *discoveryServer) GetTargets(ctx context.Context, req *pb.TargetRequest) (*pb.TargetsResponse, error) {
    // Query database for available servers
    rows, err := s.db.Query("SELECT id, address, type, players FROM servers WHERE online = true")
    if err != nil {
        return nil, err
    }
    defer rows.Close()

    var targets []*pb.Target
    for rows.Next() {
        var id, address, serverType string
        var players int
        rows.Scan(&id, &address, &serverType, &players)

        host, port := parseAddress(address)
        targets = append(targets, &pb.Target{
            Identifier: id,
            Address: &pb.Address{
                Hostname: host,
                Port: uint32(port),
            },
            Meta: []*pb.MetaEntry{
                {Key: "type", Value: serverType},
                {Key: "players", Value: fmt.Sprintf("%d", players)},
            },
        })
    }

    return &pb.TargetsResponse{Targets: targets}, nil
}
```

#### Dynamic Metadata

The gRPC adapter is perfect for dynamic metadata:

```go
// Return real-time player counts
{Key: "players", Value: fmt.Sprintf("%d", getCurrentPlayers(serverID))}

// Return server health status
{Key: "healthy", Value: fmt.Sprintf("%t", checkHealth(serverID))}

// Return dynamic capacity
{Key: "capacity", Value: fmt.Sprintf("%d", getCapacity(serverID))}
```

---

### Agones Discovery Adapter

Auto-discovers game servers in a Kubernetes cluster using Agones.

**Use when:** You're running on Kubernetes with Agones for game server orchestration.

#### Configuration

```toml
[target_discovery]
adapter = "agones"

[target_discovery.agones]
namespace = "minecraft"
label_selector = "game=minecraft,type=lobby"  # Optional
```

#### Parameters

- **`namespace`** (string, required): Kubernetes namespace to search for GameServers
- **`label_selector`** (string, optional): Label selector to filter GameServers

#### How It Works

The Agones adapter:
1. Watches for Agones `GameServer` custom resources in the specified namespace
2. Filters servers based on `label_selector` (if provided)
3. Only includes servers in `Ready` or `Allocated` state
4. Automatically updates the list when servers are added/removed/changed
5. Extracts metadata from:
   - GameServer labels
   - GameServer annotations
   - GameServer counters
   - GameServer lists

#### GameServer Metadata

Agones GameServers automatically provide metadata:

```yaml
apiVersion: "agones.dev/v1"
kind: GameServer
metadata:
  name: minecraft-lobby-1
  labels:
    game: minecraft
    type: lobby
    region: us-east
  annotations:
    version: "1.21"
    motd: "Welcome to Lobby 1"
spec:
  # ... GameServer spec
status:
  state: Ready
  address: "10.0.1.10"
  ports:
    - name: minecraft
      port: 7654
  counters:
    players:
      count: 15
      capacity: 50
```

This becomes a target:

```
Identifier: minecraft-lobby-1
Address: 10.0.1.10:7654
Metadata:
  - game: minecraft
  - type: lobby
  - region: us-east
  - version: 1.21
  - motd: Welcome to Lobby 1
  - state: Ready
  - players: 15
```

#### Label Selector Examples

```toml
# All Minecraft servers
label_selector = "game=minecraft"

# Only lobby servers
label_selector = "game=minecraft,type=lobby"

# Specific region
label_selector = "region=us-east"

# Multiple conditions
label_selector = "game=minecraft,type in (lobby,hub),region!=eu-west"
```

See [Kubernetes Label Selectors](https://kubernetes.io/docs/concepts/overview/working-with-objects/labels/) for syntax.

#### Example GameServer Fleet

```yaml
apiVersion: "agones.dev/v1"
kind: Fleet
metadata:
  name: minecraft-lobbies
  namespace: minecraft
spec:
  replicas: 5
  template:
    metadata:
      labels:
        game: minecraft
        type: lobby
    spec:
      ports:
        - name: minecraft
          containerPort: 25565
          protocol: TCP
      counters:
        players:
          count: 0
          capacity: 50
      template:
        spec:
          containers:
            - name: minecraft
              image: minecraft-server:latest
              env:
                - name: SERVER_TYPE
                  value: "lobby"
```

Passage will automatically discover all pods in this fleet.

#### Permissions Required

Passage needs Kubernetes RBAC permissions to watch GameServers:

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: passage-agones
  namespace: minecraft
rules:
  - apiGroups: ["agones.dev"]
    resources: ["gameservers"]
    verbs: ["get", "list", "watch"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: passage-agones
  namespace: minecraft
subjects:
  - kind: ServiceAccount
    name: passage
    namespace: passage
roleRef:
  kind: Role
  name: passage-agones
  apiGroup: rbac.authorization.k8s.io
```

---

## Choosing an Adapter

| Adapter | Performance | Flexibility | Complexity | Use Case |
|---------|-------------|-------------|------------|----------|
| **Fixed** | Fastest | Low | None | Static server list |
| **gRPC** | Medium | Highest | Medium | Dynamic server management |
| **Agones** | Medium | High | Low | Kubernetes + Agones |

## Best Practices

### Fixed Adapter
- Update metadata manually or via automation
- Use consistent naming conventions for identifiers
- Group servers logically with metadata

### gRPC Adapter
- Cache expensive operations in your service
- Return only healthy/available servers
- Keep response times under 50ms
- Implement proper error handling

### Agones Adapter
- Use meaningful labels for filtering
- Update GameServer counters regularly
- Use annotations for non-indexed metadata
- Set appropriate resource limits

## Combining with Target Strategies

Discovery adapters provide the *pool* of servers. Strategy adapters *select* from that pool.

Example flow:

```
1. Discovery returns: [hub-1, hub-2, survival-1]
2. Strategy filters by metadata and selects: hub-2
3. Passage transfers player to hub-2
```

See [Target Strategy Adapters](/customization/target-strategy-adapters/) for selection logic.

## Troubleshooting

### No targets returned

**Fixed Adapter:**
- Verify config syntax is correct
- Check target addresses are valid

**gRPC Adapter:**
- Test gRPC service: `grpcurl -plaintext -d '{}' localhost:3030 scrayosnet.passage.adapter.Discovery/GetTargets`
- Check service logs for errors
- Verify network connectivity

**Agones Adapter:**
- Check namespace exists: `kubectl get ns minecraft`
- Verify GameServers exist: `kubectl get gameservers -n minecraft`
- Check RBAC permissions: `kubectl auth can-i watch gameservers --as=system:serviceaccount:passage:passage -n minecraft`
- View Passage logs: `kubectl logs -n passage -l app=passage`

### Targets have incorrect addresses

- For fixed adapter: Update config
- For gRPC: Check service implementation
- For Agones: Verify GameServer status.address is correct

### Metadata not being used

Metadata must be consumed by the Target Strategy adapter. See strategy configuration for how to use metadata in selection.

## Next Steps

- Configure [Target Strategy Adapters](/customization/target-strategy-adapters/)
- Learn about [Status Adapters](/customization/status-adapters/)
- Implement [Custom gRPC Adapters](/advanced/custom-grpc-adapters/)
- Set up [Kubernetes Deployment](/setup/kubernetes/)
