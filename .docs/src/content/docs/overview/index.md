---
title: Introduction
description: Learn about Passage, the Minecraft Server Transfer Router that revolutionizes network connectivity.
---

Passage is a fast, secure, and stateless Minecraft Server Transfer Router that connects your network effortlessly—scaling infinitely without the hassles of traditional proxies.

## What is Passage?

Passage is a modern alternative to traditional Minecraft proxies like BungeeCord, Waterfall, and Velocity. Instead of maintaining persistent connections and transcoding every packet, Passage acts as a smart entrypoint that:

1. **Authenticates** connecting players with Mojang
2. **Validates** their connection and handles resource packs
3. **Routes** them to the appropriate backend server
4. **Transfers** them using Minecraft's native transfer packet
5. **Disconnects**, leaving the player directly connected to the backend

This stateless approach eliminates the proxy bottleneck entirely.

## Why Passage?

### 🚀 Performance
- No packet transcoding overhead
- Minimal latency - players connect directly to game servers
- Written in Rust for maximum efficiency

### 📈 Scalability
- Stateless architecture means horizontal scaling is trivial
- No session state to synchronize between instances
- Handle millions of connections with minimal resources

### 🔐 Security
- Full Mojang authentication support
- Native chat signing preservation
- Rate limiting to prevent connection floods

### 🔧 Flexibility
- Pluggable adapter system for status, discovery, and routing
- Support for fixed configs, HTTP/gRPC adapters, and Kubernetes/Agones
- Comprehensive observability with OpenTelemetry

### 🎯 Future-Proof
- Works with any Minecraft version 1.20.5+
- No need to update for new protocol versions
- Modern, actively maintained codebase

## Key Concepts

### Transfer Packet
Passage leverages the `Transfer (configuration)` packet introduced in Minecraft 1.20.5. This packet allows servers to transfer players to a different server address, enabling seamless routing without persistent proxy connections.

### Adapters
Passage uses a flexible adapter system with three types:
- **Status Adapters**: Provide server status information (MOTD, player count, etc.)
- **Target Discovery Adapters**: Discover available backend servers
- **Target Strategy Adapters**: Select the best server for each connecting player

### Stateless Design
Unlike traditional proxies, Passage doesn't maintain long-lived connections. Each player connection goes through authentication and routing, then Passage steps aside. This means:
- No memory overhead per connected player
- Instant recovery from Passage restarts
- No split-brain scenarios in clustered deployments

## Use Cases

Passage is ideal for:
- **Large Minecraft networks** needing to scale beyond single-proxy limitations
- **Cloud-native deployments** on Kubernetes with dynamic server discovery
- **High-availability setups** requiring zero-downtime deployments
- **Networks prioritizing chat signing** and authentication security
- **Modern architectures** wanting to eliminate proxy bottlenecks

## System Requirements

- Minecraft client version 1.20.5 or higher
- Linux, macOS, or Windows server
- Minimal resource requirements (typically <100MB RAM)
- Optional: Kubernetes cluster for Agones integration

## Next Steps

Ready to get started? Check out our [Installation Guide](/setup/installation/) to deploy Passage in minutes.
