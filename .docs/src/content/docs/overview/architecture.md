---
title: Architecture
description: Understanding Passage's architecture and the three-phase connection flow.
---

Passage is built on a modular, adapter-based architecture that separates concerns into three distinct phases. This design allows for maximum flexibility while maintaining simplicity.

## High-Level Overview

```mermaid
flowchart LR
    Player[Player] --> Passage[Passage]
    Passage --> GameServer[Game Server]

    Passage --> Status[Status Adapter]
    Passage --> Discovery[Target Discovery Adapter]
    Passage --> Strategy[Target Strategy Adapter]
```

## The Three-Phase Connection Flow

Every player connection goes through three distinct phases:

### Phase 1: Status & Authentication

When a player first connects, Passage:

1. **Receives the handshake** and status request
2. **Queries the Status Adapter** for server information (MOTD, player count, favicon)
3. **Returns the status** to the client
4. **Authenticates with Mojang** using the player's username and shared secret
5. **Encrypts the connection** using AES-128-CFB8
6. **Optionally handles resource packs** if configured

This phase ensures only legitimate players proceed to routing.

### Phase 2: Target Discovery

Once authenticated, Passage needs to know which backend servers are available:

1. **Queries the Target Discovery Adapter**
2. **Receives a list of available targets** with metadata
3. **Each target includes**:
   - Unique identifier
   - Network address (IP:port)
   - Metadata (key-value pairs like player count, server type, etc.)

Examples of discovery adapters:
- **Fixed**: Static list from configuration
- **gRPC**: Dynamic list from a custom service
- **Agones**: Auto-discovery of Kubernetes game servers

### Phase 3: Target Strategy

With the list of available servers, Passage selects the best one:

1. **Queries the Target Strategy Adapter** with:
   - List of available targets
   - Player information (UUID, username)
   - Client address and protocol version
2. **Strategy returns the selected target**
3. **Passage sends the Transfer packet** with the target address
4. **Connection closes** - player is now directly connected to the backend

Examples of strategy adapters:
- **Fixed**: Always select the first available target
- **Player Fill**: Fill servers sequentially to maximize occupancy
- **gRPC**: Custom logic (e.g., region-based, queue priority, etc.)

## Adapter System

### Why Adapters?

The adapter pattern allows Passage to remain:
- **Simple**: Core logic is unchanged regardless of your deployment
- **Flexible**: Swap adapters without modifying code
- **Extensible**: Implement custom gRPC adapters for your specific needs

### Built-in Adapters

#### Status Adapters
- **Fixed**: Static configuration (name, MOTD, favicon)
- **HTTP**: Query status from HTTP endpoint with caching
- **gRPC**: Dynamic status from custom service

#### Target Discovery Adapters
- **Fixed**: Static server list from config
- **gRPC**: Dynamic server list from custom service
- **Agones**: Kubernetes game server auto-discovery

#### Target Strategy Adapters
- **Fixed**: Simple first-available selection
- **Player Fill**: Fill servers to capacity before starting new ones
- **gRPC**: Custom selection logic

## Component Diagram

```mermaid
flowchart TB
    subgraph Passage
        subgraph Connection["Connection Handler"]
            H[Handshake]
            A[Authentication]
            E[Encryption]
            T[Transfer]
        end

        subgraph Adapters["Adapter Layer"]
            Status[Status Supplier<br/>Fixed/HTTP/gRPC]
            Discovery[Target Selector<br/>Fixed/gRPC/Agones]
            Strategy[Target Strategy<br/>Fixed/Fill/gRPC]
        end

        subgraph Support["Supporting Components"]
            RL[Rate Limiter<br/>Per-IP limits]
            Obs[Observability<br/>OpenTelemetry/Sentry]
        end

        Connection --> Status
        Connection --> Discovery
        Connection --> Strategy
    end
```

## Data Flow

### Successful Connection Flow

```mermaid
sequenceDiagram
    participant Player
    participant Passage
    participant Adapters
    participant Backend

    Player->>Passage: Handshake
    Player->>Passage: Status Request
    Passage->>Adapters: Get Status
    Adapters-->>Passage: Status Response
    Passage-->>Player: Status Response

    Player->>Passage: Login Start
    Passage-->>Player: Encryption Request
    Player->>Passage: Encryption Response
    Passage->>Adapters: Mojang Auth
    Adapters-->>Passage: Auth Success
    Passage-->>Player: Login Success

    Passage-->>Player: Transfer to Config
    Player->>Passage: Config Acknowledge
    Passage->>Adapters: Get Targets
    Adapters-->>Passage: Target List
    Passage->>Adapters: Select Target
    Adapters-->>Passage: Selected Target
    Passage-->>Player: Transfer Packet

    Player->>Backend: Connect directly to backend
```

## Stateless Design Benefits

Passage's stateless architecture means:

1. **No memory per player**: After transfer, Passage has zero memory of the player
2. **Instant restart**: Restarting Passage doesn't affect connected players
3. **Horizontal scaling**: Run multiple Passage instances with simple load balancing
4. **No synchronization**: No need to sync state between instances

## Performance Characteristics

- **Memory**: ~50-100MB base + ~10KB per concurrent connection
- **CPU**: Minimal - mostly I/O bound
- **Network**: ~5-20KB per player connection (authentication + transfer)
- **Latency**: <50ms added to connection time (depends on Mojang API)
