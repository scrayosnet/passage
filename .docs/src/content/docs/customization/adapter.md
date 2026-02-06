---
title: Adapter Overview
description: Understanding Passage's adapter system and how to choose the right adapters for your network.
---

Passage uses a flexible adapter system to separate concerns and allow customization of different aspects of player routing. This page provides an overview of the adapter architecture and helps you choose the right adapters for your needs.

## What are Adapters?

Adapters are pluggable components that implement specific functionality in Passage. They allow you to customize behavior without modifying Passage's core code.

Think of adapters like plugins: you configure which adapter to use, and Passage calls that adapter when it needs that functionality.

## The Three Adapter Types

### 1. Status Adapters

**Purpose:** Provide server status information for the multiplayer server list.

**When used:** Every time a player's client pings the server for status.

**What they provide:**
- Server name/MOTD
- Player count (online/max)
- Favicon
- Protocol version
- Secure chat enforcement

**Available adapters:**
- **Fixed**: Static configuration
- **HTTP**: Query a web API
- **gRPC**: Custom service implementation

[→ Detailed Status Adapter Documentation](/customization/status-adapters/)

### 2. Target Discovery Adapters

**Purpose:** Discover which backend Minecraft servers are available.

**When used:** Every time a player authenticates and needs to be routed.

**What they provide:**
- List of available servers
- Server addresses (IP:port)
- Server metadata (type, player count, region, etc.)

**Available adapters:**
- **Fixed**: Static server list from config
- **gRPC**: Dynamic list from custom service
- **Agones**: Auto-discovery from Kubernetes

[→ Detailed Target Discovery Documentation](/customization/target-discovery-adapters/)

### 3. Target Strategy Adapters

**Purpose:** Select which specific server to send each player to.

**When used:** After discovering available servers, before transferring the player.

**What they do:**
- Receive the list of available servers
- Apply selection logic (fill strategy, region matching, etc.)
- Return the chosen server

**Available adapters:**
- **Fixed**: Select first available server
- **Player Fill**: Fill servers to capacity sequentially
- **gRPC**: Custom selection logic

[→ Detailed Target Strategy Documentation](/customization/target-strategy-adapters/)

## How Adapters Work Together

Every player connection uses all three adapter types:

```
Player Connects
      │
      ▼
┌──────────────────┐
│ Status Adapter   │ ← Provides server list info
└────────┬─────────┘
         │
         ▼
┌──────────────────┐
│ Authentication   │
│ & Encryption     │
└────────┬─────────┘
         │
         ▼
┌──────────────────┐
│ Target Discovery │ ← Finds available servers
│ Adapter          │
└────────┬─────────┘
         │
         ▼
┌──────────────────┐
│ Target Strategy  │ ← Selects best server
│ Adapter          │
└────────┬─────────┘
         │
         ▼
    Transfer Player
```

## Choosing Adapters

### Decision Matrix

| Scenario | Status | Discovery | Strategy |
|----------|--------|-----------|----------|
| Single static server | Fixed | Fixed | Fixed |
| Multiple static servers, fill sequentially | Fixed | Fixed | Player Fill |
| Kubernetes with Agones | Fixed/HTTP | Agones | Player Fill |
| Full custom control | gRPC | gRPC | gRPC |
| Dynamic status, static servers | HTTP | Fixed | Fixed |
| Multi-region routing | gRPC | gRPC | gRPC |

### By Complexity

#### Simple (No Code)
- **Status**: Fixed
- **Discovery**: Fixed
- **Strategy**: Fixed or Player Fill

Best for: Small networks with a few static servers.

#### Intermediate (HTTP API)
- **Status**: HTTP
- **Discovery**: Fixed or Agones
- **Strategy**: Player Fill

Best for: Networks with dynamic status or Kubernetes deployments.

#### Advanced (Custom gRPC)
- **Status**: gRPC
- **Discovery**: gRPC
- **Strategy**: gRPC

Best for: Complex networks with custom routing logic, multi-region setups, or queue systems.

## Common Configurations

### Basic Static Setup

```toml
[status]
adapter = "fixed"

[target_discovery]
adapter = "fixed"

[target_strategy]
adapter = "fixed"
```

**Use case:** Development, single-server networks

### Progressive Fill Setup

```toml
[status]
adapter = "fixed"

[target_discovery]
adapter = "fixed"
[[target_discovery.fixed.targets]]
identifier = "hub-1"
address = "10.0.1.10:25565"
meta = { type = "hub", players = "0" }

[target_strategy]
adapter = "player_fill"
[target_strategy.player_fill]
field = "players"
max_players = 50
```

**Use case:** Multiple lobbies/hubs, consolidate players for better experience

### Kubernetes/Agones Setup

```toml
[status]
adapter = "http"
[status.http]
address = "http://status-service/status"
cache_duration = 5

[target_discovery]
adapter = "agones"
[target_discovery.agones]
namespace = "minecraft"

[target_strategy]
adapter = "player_fill"
[target_strategy.player_fill]
field = "players"
max_players = 50
```

**Use case:** Cloud-native deployments with auto-scaling game servers

### Full Custom Control

```toml
[status]
adapter = "grpc"
[status.grpc]
address = "http://status-service:3030"

[target_discovery]
adapter = "grpc"
[target_discovery.grpc]
address = "http://discovery-service:3030"

[target_strategy]
adapter = "grpc"
[target_strategy.grpc]
address = "http://strategy-service:3030"
```

**Use case:** Complex requirements, custom business logic, multi-region routing

## Adapter Independence

Adapters are independent and can be mixed:

```toml
# Use fixed status, but dynamic discovery
[status]
adapter = "fixed"

[target_discovery]
adapter = "agones"

[target_strategy]
adapter = "grpc"
```

This flexibility allows you to:
- Start simple and add complexity where needed
- Use different adapters for different concerns
- Gradually migrate to more advanced setups

## Performance Considerations

### Fastest Configuration
- Status: Fixed
- Discovery: Fixed
- Strategy: Fixed

**Latency:** <5ms per connection

### Moderate Performance
- Status: HTTP (cached)
- Discovery: Agones
- Strategy: Player Fill

**Latency:** 10-50ms per connection

### Custom Performance
- Status: gRPC
- Discovery: gRPC
- Strategy: gRPC

**Latency:** Depends on your implementation (aim for <50ms total)

## Extending with gRPC

All three adapter types support custom gRPC implementations. This allows you to:

- Integrate with existing infrastructure
- Implement complex business logic
- Use any programming language (Go, Java, Python, Node.js, etc.)
- Maintain separation of concerns

See [Custom gRPC Adapters](/advanced/custom-grpc-adapters/) for implementation guides.

## Best Practices

### Start Simple
Begin with Fixed adapters and migrate to more complex ones as needed.

### Keep Adapters Fast
- Status adapters should respond in <10ms
- Discovery adapters should respond in <50ms
- Strategy adapters should respond in <10ms

### Use Appropriate Caching
- HTTP Status adapter has built-in caching
- Implement caching in your gRPC services
- Agones adapter caches automatically via watch

### Monitor Adapter Performance
Use OpenTelemetry to track:
- Adapter response times
- Adapter error rates
- Selection distribution (for strategy adapters)

### Fail Gracefully
- If Status adapter fails, use a default status
- If Discovery returns empty, disconnect with a friendly message
- If Strategy returns none, disconnect with appropriate error

## Troubleshooting

### "No adapter configured" error
Check that you've set the `adapter` field for all three adapter types.

### Adapters not being called
Enable debug logging: `RUST_LOG=debug passage`

### Slow connections
Check adapter response times in logs/metrics. Each adapter should be fast (<50ms).

### gRPC connection refused
Verify the address includes scheme (`http://` or `https://`).

## Next Steps

- Configure [Status Adapters](/customization/status-adapters/)
- Set up [Target Discovery](/customization/target-discovery-adapters/)
- Implement [Target Strategy](/customization/target-strategy-adapters/)
- Learn about [Custom gRPC Adapters](/advanced/custom-grpc-adapters/)
