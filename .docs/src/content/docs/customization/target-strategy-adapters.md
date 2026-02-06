---
title: Target Strategy Adapters
description: Configure how Passage selects which server to send each player to.
---

Target Strategy adapters implement the selection logic that chooses which backend server a player should be sent to from the list of available targets provided by the Discovery adapter.

## Overview

After discovering available servers, Passage uses the Strategy adapter to select the best one for each player. The adapter receives:
- List of available targets (from Discovery)
- Player information (UUID, username)
- Connection context (client IP, server address, protocol version)

It returns the selected target, or none if no suitable server is available.

## Available Adapters

### Fixed Strategy Adapter

Always selects the first available target from the list.

**Use when:** You have a single server or want simple round-robin via DNS/load balancer.

#### Configuration

```toml
[target_strategy]
adapter = "fixed"
```

#### Behavior

- Returns the first target in the discovery list
- If no targets are available, returns none (player is disconnected)
- Deterministic and predictable
- Zero configuration

#### Use Cases

- Single backend server
- Load balancer handles distribution
- Simple testing/development setup

---

### Player Fill Strategy Adapter

Fills servers sequentially to capacity before starting new ones.

**Use when:** You want to consolidate players for better gameplay experience and resource efficiency.

#### Configuration

```toml
[target_strategy]
adapter = "player_fill"

[target_strategy.player_fill]
field = "players"  # Metadata field containing player count
max_players = 50   # Maximum players before considering server full

# Optional: Filter and route based on server host
[[target_strategy.player_fill.target_filters]]
server_host = "lobby.example.com"
identifier = "lobby-1"  # Only match servers with this identifier
meta = { type = "lobby" }  # Only match servers with this metadata
allow_list = ["Steve", "8667ba71-b85a-4004-af54-457a9734eed7"]  # Whitelist specific players

[[target_strategy.player_fill.target_filters]]
server_host = "survival.example.com"
meta = { type = "survival", difficulty = "hard" }
```

#### Parameters

- **`field`** (string, required): Metadata key containing current player count
- **`max_players`** (number, required): Max players per server
- **`target_filters`** (array, optional): Rules for filtering targets

#### Target Filter Parameters

- **`server_host`** (string, required): Match incoming connection host
- **`identifier`** (string, optional): Match specific target identifier
- **`meta`** (map, optional): Match targets with these metadata values
- **`allow_list`** (array, optional): Only allow these usernames/UUIDs

#### How It Works

1. Filter targets based on `target_filters` (if configured)
2. Parse player count from the `field` metadata
3. Exclude servers at or above `max_players`
4. Select the fullest server below capacity
5. If no suitable server exists, return none

#### Example: Progressive Filling

With this configuration:

```toml
[target_strategy.player_fill]
field = "players"
max_players = 50
```

And these targets:

```
hub-1: players=45
hub-2: players=12
hub-3: players=0
```

Players will be sent to **hub-1** (45/50) until it reaches 50, then **hub-2** (12/50), and finally **hub-3**.

#### Example: Multi-Domain Routing

```toml
[target_strategy.player_fill]
field = "players"
max_players = 100

[[target_strategy.player_fill.target_filters]]
server_host = "lobby.example.com"
meta = { type = "lobby" }

[[target_strategy.player_fill.target_filters]]
server_host = "survival.example.com"
meta = { type = "survival" }

[[target_strategy.player_fill.target_filters]]
server_host = "creative.example.com"
meta = { type = "creative" }
```

Now:
- `lobby.example.com` → routes to lobby servers
- `survival.example.com` → routes to survival servers
- `creative.example.com` → routes to creative servers

#### Example: VIP Whitelist

```toml
[target_strategy.player_fill]
field = "players"
max_players = 50

# VIP server - only specific players
[[target_strategy.player_fill.target_filters]]
server_host = "vip.example.com"
meta = { type = "vip" }
allow_list = [
    "Notch",
    "c06f8906-4c8a-4911-9c29-ea1dbd1aab82",  # UUID
    "jeb_"
]

# Regular server - everyone else
[[target_strategy.player_fill.target_filters]]
server_host = "play.example.com"
meta = { type = "regular" }
```

#### Player Count Source

The `field` value must come from target metadata:

**Fixed Discovery:**
```toml
[[target_discovery.fixed.targets]]
identifier = "hub-1"
address = "10.0.1.10:25565"
meta = { type = "hub", players = "15" }  # Must be updated externally
```

**gRPC Discovery:**
```go
// Return live player counts
Meta: []*pb.MetaEntry{
    {Key: "players", Value: fmt.Sprintf("%d", getLivePlayerCount(serverID))},
}
```

**Agones Discovery:**
```yaml
# Automatically from GameServer counters
spec:
  counters:
    players:
      count: 15  # Passage reads this automatically
```

---

### gRPC Strategy Adapter

Implements custom selection logic in an external gRPC service.

**Use when:** You need complex routing logic (region-based, queue systems, skill-based matchmaking, etc.).

#### Configuration

```toml
[target_strategy]
adapter = "grpc"

[target_strategy.grpc]
address = "http://strategy-service:3030"
```

#### Parameters

- **`address`** (string, required): Address of the gRPC strategy service

#### gRPC Service Definition

Your service must implement the `Strategy` service:

```protobuf
syntax = "proto3";

service Strategy {
    rpc SelectTarget(SelectRequest) returns (SelectResponse);
}

message SelectRequest {
    Address client_address = 1;
    Address server_address = 2;
    uint64 protocol = 3;
    string username = 4;
    string user_id = 5;
    repeated Target targets = 6;
}

message SelectResponse {
    optional Target target = 1;
}
```

#### Request Context

Your service receives:
- **client_address**: Player's IP and port
- **server_address**: Domain/IP the player connected to
- **protocol**: Minecraft protocol version
- **username**: Player's username
- **user_id**: Player's UUID
- **targets**: All available servers from Discovery

#### Example: Region-Based Selection

```go
package main

import (
    "context"
    "strings"
    pb "github.com/scrayosnet/passage/proto/adapter"
}

type strategyServer struct {
    pb.UnimplementedStrategyServer
    geoip GeoIPService
}

func (s *strategyServer) SelectTarget(ctx context.Context, req *pb.SelectRequest) (*pb.SelectResponse, error) {
    // Get player's region from IP
    playerRegion := s.geoip.GetRegion(req.ClientAddress.Hostname)

    // Find servers in player's region
    for _, target := range req.Targets {
        targetRegion := getMetadata(target, "region")
        if targetRegion == playerRegion {
            return &pb.SelectResponse{Target: target}, nil
        }
    }

    // Fallback to first available server
    if len(req.Targets) > 0 {
        return &pb.SelectResponse{Target: req.Targets[0]}, nil
    }

    return &pb.SelectResponse{Target: nil}, nil
}

func getMetadata(target *pb.Target, key string) string {
    for _, entry := range target.Meta {
        if entry.Key == key {
            return entry.Value
        }
    }
    return ""
}
```

#### Example: Skill-Based Matchmaking

```go
func (s *strategyServer) SelectTarget(ctx context.Context, req *pb.SelectRequest) (*pb.SelectResponse, error) {
    // Get player's skill rating from database
    playerSkill := s.db.GetPlayerSkill(req.UserId)

    // Find server with similar skill level
    var bestTarget *pb.Target
    smallestSkillDiff := float64(999999)

    for _, target := range req.Targets {
        avgSkill := parseFloat(getMetadata(target, "avg_skill"))
        diff := math.Abs(playerSkill - avgSkill)

        if diff < smallestSkillDiff {
            smallestSkillDiff = diff
            bestTarget = target
        }
    }

    return &pb.SelectResponse{Target: bestTarget}, nil
}
```

#### Example: Queue System

```go
func (s *strategyServer) SelectTarget(ctx context.Context, req *pb.SelectRequest) (*pb.SelectResponse, error) {
    // Check if player is in queue
    queueEntry := s.queue.Get(req.UserId)
    if queueEntry == nil {
        // Add to queue
        s.queue.Add(req.UserId, req.Targets)
        return &pb.SelectResponse{Target: nil}, nil // Disconnect with queue message
    }

    // Check if server is ready
    server := s.queue.GetAssignedServer(req.UserId)
    if server != nil {
        // Find matching target
        for _, target := range req.Targets {
            if target.Identifier == server.ID {
                s.queue.Remove(req.UserId)
                return &pb.SelectResponse{Target: target}, nil
            }
        }
    }

    // Still waiting
    return &pb.SelectResponse{Target: nil}, nil
}
```

---

## Choosing an Adapter

| Adapter | Performance | Flexibility | Complexity | Use Case |
|---------|-------------|-------------|------------|----------|
| **Fixed** | Fastest | None | None | Single server or external LB |
| **Player Fill** | Fast | Medium | Low | Consolidate players |
| **gRPC** | Medium | Highest | High | Custom routing logic |

## Best Practices

### Fixed Adapter
- Use with load balancer for distribution
- Perfect for single-server networks
- Combine with DNS round-robin

### Player Fill Adapter
- Keep player counts updated in metadata
- Set `max_players` slightly below server capacity (e.g., 48/50)
- Use target filters for multi-domain routing
- Consider server warmup time in capacity planning

### gRPC Adapter
- Keep response times under 50ms
- Implement fallback logic (always return a server if available)
- Cache expensive operations (database lookups, API calls)
- Log selection decisions for debugging

## Selection Flow Example

Complete example with all three adapter types:

```
┌─────────────────────────────────────────────────────┐
│ Player connects to play.example.com                 │
└──────────────────┬──────────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────────┐
│ Discovery Adapter returns:                          │
│   - hub-1 (10.0.1.10:25565) {players=45, type=hub} │
│   - hub-2 (10.0.1.11:25565) {players=12, type=hub} │
│   - hub-3 (10.0.1.12:25565) {players=0, type=hub}  │
└──────────────────┬──────────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────────┐
│ Strategy Adapter (player_fill):                     │
│   1. Filter: type=hub (all match)                  │
│   2. Exclude: players >= 50 (none excluded)        │
│   3. Select: Fullest = hub-1 (45 players)          │
└──────────────────┬──────────────────────────────────┘
                   │
                   ▼
┌─────────────────────────────────────────────────────┐
│ Transfer player to 10.0.1.10:25565                  │
└─────────────────────────────────────────────────────┘
```

## Troubleshooting

### All players going to the same server

**Fixed Adapter:** Expected behavior. Use Player Fill or gRPC for distribution.

**Player Fill Adapter:**
- Verify `field` metadata is correct and updated
- Check `max_players` is set appropriately
- Enable debug logging to see selection logic

**gRPC Adapter:**
- Check service logs for selection decisions
- Verify targets are being evaluated correctly
- Test with `grpcurl` manually

### Players disconnected with "No server available"

- Check Discovery adapter is returning targets
- Verify Strategy adapter is selecting a target
- Check target filters aren't excluding all servers
- For Player Fill: Ensure at least one server is below max_players

### Target filters not working

- Verify `server_host` matches exactly
- Check metadata keys and values match exactly
- Use debug logging to see filter evaluation
- Test with a simple filter first

### Poor performance

**Player Fill:**
- Cache metadata parsing
- Keep `target_filters` simple

**gRPC:**
- Profile your service for bottlenecks
- Cache database/API results
- Use connection pooling
- Return quickly (<50ms)

## Next Steps

- Configure [Target Discovery Adapters](/customization/target-discovery-adapters/)
- Learn about [Status Adapters](/customization/status-adapters/)
- Implement [Custom gRPC Adapters](/advanced/custom-grpc-adapters/)
- Review [Configuration Reference](/reference/configuration-reference/)
