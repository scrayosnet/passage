---
title: Target Discovery Adapter
description: Configure how Passage discovers available backend servers.
sidebar:
    order: 4
---

The discovery adapter produces the initial list of backend servers (targets) for a route. After discovery, the optional [actions pipeline](/adapters/discovery-actions/) can filter, reorder, or transform the target list before the first target is selected.

Discovery is configured in `routes[].discovery`:

```yaml
routes:
- hostname: "mc.example.net"
  discovery:
    type: dns_discovery          # The discovery adapter type
    domain: "servers.example.net" # Adapter-specific fields
    record_type: srv
    actions:                      # Optional actions pipeline
    - type: meta_filter
      rules:
      - key: "status"
        op: equals
        value: "online"
```

The `type` field selects the discovery adapter. All remaining fields at the same level configure that adapter. The `actions` array is separate and always optional.

---

## Fixed Discovery (Default)

Returns a static list of targets defined in the configuration. Best for simple setups or development.

```yaml
discovery:
  type: fixed_discovery
  targets:
  - identifier: "hub-1"
    address: "10.0.1.10:25565"
    meta:
      type: "hub"
      players: "45"
  - identifier: "hub-2"
    address: "10.0.1.11:25565"
    meta:
      type: "hub"
      players: "38"
  - identifier: "survival-1"
    address: "10.0.2.10:25565"
    meta:
      type: "survival"
      players: "12"
```

### Target Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `identifier` | string | `""` | Unique name for the target (used in logging) |
| `address` | string | `""` | Network address in `host:port` format |
| `priority` | integer | `0` | Priority for ordering (lower = higher priority) |
| `meta` | map of strings | `{}` | Arbitrary key-value metadata |

### Metadata

Metadata is a flat map of string key-value pairs attached to each target. It is used by [discovery actions](/adapters/discovery-actions/) like `meta_filter` and `player_fill_strategy` to make routing decisions. Common metadata fields:

| Key | Example | Used By |
|-----|---------|---------|
| `type` | `"hub"`, `"survival"` | `meta_filter` |
| `status` | `"online"`, `"maintenance"` | `meta_filter` |
| `players` | `"45"` | `player_fill_strategy` |
| `region` | `"eu-west"` | `meta_filter` |

:::note
With `fixed_discovery`, metadata values are static. For dynamic metadata that reflects real-time server state (player counts, status), use `dns_discovery`, `agones_discovery`, or `grpc_discovery`.
:::

---

## DNS Discovery

Discovers targets by querying DNS records. Supports both **SRV** and **A/AAAA** record types. The DNS records are periodically re-queried to keep the target list up to date.

### SRV Records

SRV records include both host and port information, making them the recommended choice:

```yaml
discovery:
  type: dns_discovery
  domain: "_minecraft._tcp.example.net"
  record_type: srv
  refresh_interval: 30
```

Each SRV record becomes a target with the resolved host and port.

### A/AAAA Records

For A or AAAA records, you must specify a port since these record types only contain IP addresses:

```yaml
discovery:
  type: dns_discovery
  domain: "mc.example.net"
  record_type: a
  port: 25565
  refresh_interval: 30
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `domain` | string | `""` | The DNS domain to query |
| `record_type` | string | `"srv"` | Record type: `srv` or `a` |
| `port` | integer | `25565` | Port to use with A/AAAA records (ignored for SRV) |
| `refresh_interval` | integer | `30` | How often to re-query DNS, in seconds |

:::tip[When to Use DNS Discovery]
DNS discovery works well when your backend servers are registered in DNS (e.g., via Kubernetes Services, Consul, or manual DNS entries). SRV records are preferred because they carry port information and priority.
:::

---

## Agones Discovery

Discovers targets through [Agones](https://agones.dev/) GameServer allocations in a Kubernetes cluster. This is the recommended discovery method for Kubernetes deployments using Agones for game server orchestration.

```yaml
discovery:
  type: agones_discovery
  namespace: "minecraft"
  selectors:
  - matchLabels:
      game: "minecraft"
      type: "lobby"
  scheduling: "Packed"
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `namespace` | string | `null` | Kubernetes namespace to allocate in |
| `selectors` | array | `[]` | Label selectors for GameServer matching |
| `priorities` | array | `[]` | Priority rules for allocation ordering |
| `scheduling` | string | `null` | Scheduling strategy (`"Packed"` or `"Distributed"`) |
| `metadata` | object | `null` | Metadata to apply to the allocation |
| `backoff` | object | *(see below)* | Exponential backoff configuration |

### Template Variables

Selector and metadata fields support template variables that are replaced at allocation time:

| Variable | Description |
|----------|-------------|
| `{{ .Client.ProtocolVersion }}` | Client's Minecraft protocol version |
| `{{ .Client.ServerAddress }}` | Hostname the client connected to |
| `{{ .Client.ServerPort }}` | Port the client connected to |
| `{{ .Client.Address }}` | Client's IP address |
| `{{ .Request.TraceId }}` | OpenTelemetry trace ID for the request |

### Example with Templates

```yaml
discovery:
  type: agones_discovery
  namespace: "minecraft"
  selectors:
  - matchLabels:
      game: "minecraft"
  metadata:
    labels:
      lastClient: "{{ .Client.Address }}"
  scheduling: "Packed"
  backoff:
    initial_interval: 500
    max_interval: 5000
    max_elapsed_time: 30000
    multiplier: 1.5
    randomization_factor: 0.5
```

:::note[RBAC Requirements]
The Agones discovery adapter requires Kubernetes RBAC permissions to create `GameServerAllocation` resources. See the [Kubernetes Guide](/setup/kubernetes/) for the required ClusterRole configuration.
:::

---

## gRPC Discovery

Delegates target discovery to an external gRPC service. This gives you full control over how targets are discovered and what metadata is attached to them.

```yaml
discovery:
  type: grpc_discovery
  address: "http://discovery-service:50051"
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `address` | string | `""` | The gRPC service endpoint URL |

### gRPC Service Definition

The service must implement the `Discovery` service from `discovery.proto`:

```protobuf
service Discovery {
    rpc GetTargets(TargetRequest) returns (TargetResponse);
}
```

The `TargetRequest` includes:
- `client` (`ClientInfo`): Client address, server address, protocol version
- `player` (`PlayerInfo`): Player name and UUID

The `TargetResponse` returns a list of `Target` messages, each with an identifier, address, priority, and metadata.

See the [gRPC Protocol Reference](/reference/grpc-protocol/) for full message definitions and the [Custom gRPC Adapters](/advanced/grpc-adapters/) guide for implementation examples.

---

## What Happens After Discovery

After the discovery adapter produces a target list:

1. The [actions pipeline](/adapters/discovery-actions/) processes the list (if configured)
2. The **first target** in the resulting list is selected
3. The player is transferred to that target

If no targets remain after processing, the player is disconnected with the `disconnect_no_target` message.
