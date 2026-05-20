---
title: Customization Overview
description: Learn how to customize Passage for your Minecraft network's specific needs.
sidebar:
  order: 5
---

Passage is designed to be highly customizable while maintaining simplicity. This page provides an overview of customization options and helps you choose the right approach for your network.

## Start Simple, Scale Complexity

Passage follows a progressive enhancement philosophy:

1. **Start with fixed adapters** -- Get running quickly with static configuration
2. **Add dynamic elements** -- Introduce DNS discovery, HTTP status, or Agones as needed
3. **Implement custom logic** -- Use gRPC adapters for complex requirements
4. **Monitor and optimize** -- Add observability and fine-tune performance

You don't need to use all features at once. Many successful networks run with just fixed adapters.

## What Can Be Customized?

### Core Settings
- Network binding address and port
- Connection timeouts
- Rate limiting (per-IP connection flood protection)
- PROXY protocol support for load balancers
- Authentication cookie expiry

[Configuration Reference](/reference/configuration/)

### Per-Route Adapters

Each route independently configures five adapter categories:

| Category | What It Controls | Options |
|----------|-----------------|---------|
| **Status** | Server list response (MOTD, player count, favicon) | `fixed`, `http`, `grpc` |
| **Authentication** | Player identity verification | `mojang`, `disabled`, `fixed`, `grpc` |
| **Discovery** | How backend servers are found | `fixed_discovery`, `dns_discovery`, `agones_discovery`, `grpc_discovery` |
| **Discovery Actions** | How the target list is filtered/ordered | `meta_filter`, `player_allow_filter`, `player_block_filter`, `player_fill_strategy`, `grpc` |
| **Localization** | Disconnect message translations | `fixed`, `grpc` |

[Adapter Overview](/adapters/)

### Observability
- OpenTelemetry traces, metrics, and logs
- Sentry error tracking
- Structured logging with configurable levels

[Monitoring Guide](/advanced/observability/)

## Common Scenarios

### Small Static Network

2-3 fixed servers, simple routing:

```yaml
routes:
- hostname: "mc.example.net"
  status:
    type: fixed
    name: "My Network"
  discovery:
    type: fixed_discovery
    targets:
    - identifier: "lobby"
      address: "10.0.0.10:25565"
```

**Complexity:** None -- pure configuration.

### Multiple Lobbies with Load Balancing

Fill the fullest server below capacity:

```yaml
routes:
- hostname: "mc.example.net"
  discovery:
    type: fixed_discovery
    targets:
    - identifier: "lobby-1"
      address: "10.0.1.10:25565"
      meta: { players: "45" }
    - identifier: "lobby-2"
      address: "10.0.1.11:25565"
      meta: { players: "38" }
    actions:
    - type: player_fill_strategy
      field: "players"
      max_players: 50
```

**Complexity:** Low -- requires updating player counts in metadata (or use DNS/Agones for dynamic data).

### DNS Discovery with Filtering

Automatic server discovery with metadata-based filtering:

```yaml
routes:
- hostname: "mc.example.net"
  discovery:
    type: dns_discovery
    domain: "servers.example.net"
    record_type: srv
    actions:
    - type: meta_filter
      rules:
      - key: "status"
        op: equals
        value: "online"
    - type: player_fill_strategy
      field: "players"
      max_players: 50
```

**Complexity:** Medium -- requires DNS infrastructure.

### Kubernetes with Agones

Cloud deployment with dynamic game servers:

```yaml
routes:
- hostname: "mc.example.net"
  status:
    type: http
    address: "http://status-service.minecraft/status"
    cache_duration: 30
  discovery:
    type: agones_discovery
    namespace: "minecraft"
    selectors:
    - matchLabels:
        game: "minecraft"
        type: "lobby"
    scheduling: "Packed"
```

**Complexity:** Medium -- requires Kubernetes and Agones setup. See [Kubernetes Guide](/setup/kubernetes/).

### Multi-Region with Custom Routing

Full gRPC adapter stack for complex logic:

```yaml
routes:
- hostname: "mc.example.net"
  status:
    type: grpc
    address: "http://status-service:50051"
  discovery:
    type: grpc_discovery
    address: "http://discovery-service:50051"
    actions:
    - type: grpc
      name: "region-router"
      address: "http://router-service:50051"
```

**Complexity:** High -- requires custom gRPC services. See [Custom gRPC Adapters](/advanced/grpc-adapters/).

## Best Practices

- **Choose the simplest adapter** that meets your needs
- **Keep adapter response times under 50ms** -- slow adapters delay player connections
- **Enable rate limiting** in production
- **Start with fixed adapters** and upgrade as your needs grow
- **Monitor adapter performance** with OpenTelemetry
- **Test configuration changes** in a staging environment before production
- **Use descriptive `name` fields** on discovery actions for easier debugging
