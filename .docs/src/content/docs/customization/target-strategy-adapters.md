---
title: Target Strategy Adapters
description: Configure how Passage selects which server to send each player to.
---

Strategy adapters select which backend server to send each player to from the discovered targets.

## Fixed Adapter

Always selects the first available target.

```toml
[target_strategy]
adapter = "fixed"
```

## Player Fill Adapter

Fills servers to capacity sequentially.

```toml
[target_strategy]
adapter = "player_fill"

[target_strategy.player_fill]
field = "players"  # Metadata field containing player count
max_players = 50

# Optional: Filter by domain and metadata
[[target_strategy.player_fill.target_filters]]
server_host = "lobby.example.com"
meta = { type = "lobby" }

[[target_strategy.player_fill.target_filters]]
server_host = "survival.example.com"
meta = { type = "survival" }
```

**How it works:** Selects the fullest server below `max_players` capacity.

**Player count source:** The `field` metadata must be provided by the discovery adapter (static in Fixed, dynamic in gRPC/Agones).

## gRPC Adapter

Custom gRPC service for complex routing (region-based, skill-based matchmaking, queue systems, etc.).

```toml
[target_strategy]
adapter = "grpc"

[target_strategy.grpc]
address = "http://strategy-service:3030"
```

See [Custom gRPC Adapters](/advanced/custom-grpc-adapters/) for implementation examples.
