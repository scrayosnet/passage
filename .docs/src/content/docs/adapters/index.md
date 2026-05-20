---
title: Adapter Overview
description: Understanding Passage's adapter system and how to choose the right adapters for your network.
sidebar:
    order: 0
---

Passage uses a pluggable **adapter system** to customize every aspect of how player connections are handled. Each [route](/reference/configuration/#routes) independently configures its own set of adapters, so you can run entirely different configurations for different hostnames.

## Adapter Categories

Every route has four adapter categories:

### Status Adapter

Controls what players see in the Minecraft server list (MOTD, player count, version, favicon).

| Type | Description |
|------|-------------|
| `fixed` (default) | Returns a static, preconfigured status response |
| `http` | Fetches status from an HTTP endpoint with caching |
| `grpc` | Delegates to an external gRPC service |

[Status Adapter Reference →](/adapters/status/)

---

### Authentication Adapter

Verifies player identity before routing them to a backend server.

| Type | Description |
|------|-------------|
| `mojang` (default) | Standard Mojang/Microsoft authentication |
| `disabled` | Skips authentication entirely (testing only) |
| `fixed` | Uses a hardcoded player profile for all connections |
| `grpc` | Delegates to an external gRPC service |

[Authentication Adapter Reference →](/adapters/authentication/)

---

### Discovery Adapter + Actions Pipeline

Finds available backend servers and selects one for the player. This is a two-part system:

1. A **discovery adapter** produces an initial list of targets
2. An **actions pipeline** filters, reorders, or transforms the list sequentially

**Discovery adapters:**

| Type | Description |
|------|-------------|
| `fixed_discovery` (default) | Returns a static list of targets |
| `dns_discovery` | Discovers targets via DNS SRV or A/AAAA records |
| `agones_discovery` | Discovers targets through Agones GameServer allocations |
| `grpc_discovery` | Delegates target discovery to a gRPC service |

**Actions** (applied in order after discovery):

| Type | Description |
|------|-------------|
| `meta_filter` | Filters targets based on metadata key-value pairs |
| `player_allow_filter` | Whitelists players by username, regex, or UUID |
| `player_block_filter` | Blacklists players by username, regex, or UUID |
| `player_fill_strategy` | Reorders targets to fill the fullest server first |
| `grpc` | Delegates action logic to an external gRPC service |

[Discovery Adapter Reference →](/adapters/target-discovery/) | [Discovery Actions Reference →](/adapters/discovery-actions/)

---

### Localization Adapter

Provides translated disconnect messages based on the player's client locale.

| Type | Description |
|------|-------------|
| `fixed` (default) | Returns messages from a static configuration map |
| `grpc` | Delegates to an external gRPC service |

[Localization Reference →](/advanced/localization/)

---

## Common Configurations

| Use Case | Status | Auth | Discovery | Actions                                |
|----------|--------|------|-----------|----------------------------------------|
| Single server | `fixed` | `mojang` | `fixed_discovery` | --                                     |
| Multiple lobbies, fill evenly | `fixed` | `mojang` | `fixed_discovery` | `player_fill_strategy`                 |
| DNS-based with filtering | `http` | `mojang` | `dns_discovery` | `meta_filter` + `player_fill_strategy` |
| Kubernetes + Agones | `http` | `mojang` | `agones_discovery` | --                                     |
| Custom routing logic | `grpc` | `grpc` | `grpc_discovery` | `grpc`                                 |
| Whitelisted beta server | `fixed` | `mojang` | `fixed_discovery` | `player_allow_filter`                  |

## Quick Example

A route combining DNS discovery with metadata filtering and player fill:

```yaml
routes:
- hostname: "mc.example.net"
  status:
    type: http
    address: "https://status.example.net/status"
    cache_duration: 30
  authentication:
    type: mojang
  discovery:
    type: dns_discovery
    domain: "servers.example.net"
    record_type: srv
    actions:
    - type: meta_filter
      name: "online-filter"
      rules:
      - key: "status"
        op: equals
        value: "online"
    - type: player_fill_strategy
      name: "fill-strategy"
      field: "players"
      max_players: 50
```

For implementation examples using custom gRPC adapters, see [Custom gRPC Adapters](/advanced/grpc-adapters/).
