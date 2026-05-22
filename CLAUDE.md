# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is Passage

Passage is a Minecraft network transfer router written in Rust. Rather than acting as a traditional proxy (BungeeCord/Velocity), it validates players, handles authentication and resource pack installation, then sends them a Minecraft transfer packet (1.20.5+) to redirect them to a backend server — dropping the connection immediately after. This requires no ongoing transcription of Minecraft packets.

## Commands

```bash
# Build
cargo build

# Test (all crates, all features)
cargo test --workspace --verbose --all-features

# Run a single test
cargo test --package <crate-name> <test_name>

# Lint
cargo clippy --workspace --all-features

# Format check
cargo fmt --all -- --check

# Format fix
cargo fmt --all

# Documentation
cargo doc --workspace --no-deps

# Unused dependency check
cargo machete --with-metadata

# Security audit
cargo deny check
cargo audit
```

## Workspace Structure

A 7-crate workspace. The crates have clear layering:

| Crate                     | Role                                                                             |
|---------------------------|----------------------------------------------------------------------------------|
| `passage`                 | Binary entry point: wires adapters together, loads config, starts the server     |
| `passage-protocol`        | Core TCP listener, Minecraft protocol state machine, connection handling, crypto |
| `passage-packets`         | Minecraft packet definitions and binary serialization/deserialization            |
| `passage-adapters`        | Adapter traits + built-in implementations (Fixed, Disabled)                      |
| `passage-adapters-grpc`   | gRPC implementations of auth and discovery adapter traits                        |
| `passage-adapters-http`   | HTTP-based Mojang authentication adapter                                         |
| `passage-adapters-agones` | Kubernetes Agones game server discovery adapter                                  |
| `passage-adapters-dns`    | DNS SRV record discovery adapter                                                 |

## Architecture

### Connection Flow

```
TCP Connection → Handshake packet → match hostname to Route
  → StatusAdapter (MOTD/ping)
  → AuthenticationAdapter (validate Mojang profile)
  → LocalizationAdapter (resource packs during configuration phase)
  → DiscoveryActionAdapter chain (filter/select backend server):
      DiscoveryAdapter → MetaFilter → AllowFilter → BlockFilter → FillStrategy
  → Send Transfer packet → Drop connection
```

### Adapter System

The central extensibility mechanism. `passage-adapters` defines five traits:
- `StatusAdapter` — server status for ping responses
- `AuthenticationAdapter` — validates players (Mojang, Fixed, Disabled, gRPC)
- `DiscoveryAdapter` — fetches available backend targets
- `DiscoveryActionAdapter` — wraps DiscoveryAdapter with filter/routing logic
- `LocalizationAdapter` — localizes disconnect messages

In `passage/src/adapter/mod.rs`, these are implemented as enums (`DynAuthenticationAdapter`, etc.) that dispatch to concrete implementations. Which implementations are compiled in is controlled by Cargo features: `adapters-grpc`, `adapters-http`, `adapters-agones`, `adapters-dns` (all on by default).

### Route Matching

`Routes<Stat, Disc, Auth, Loca>` in `passage-protocol` is parameterized by adapter types. Each `Route` holds a hostname regex pattern and one instance of each adapter, allowing different auth/discovery logic per virtual hostname.

### Configuration

Loaded in priority order (highest to lowest):
1. Environment variables with `PASSAGE_` prefix
2. Auth secret file (optional)
3. Config file (`config/config`, optional)
4. Hardcoded defaults

Uses the `config` crate. See `passage/src/config.rs`.

### Protocol State Machine

Connections progress through states: `Handshake → Status | Login → Configuration → Transfer`. The configuration phase handles resource pack delivery with keep-alive packets sent on 16-second intervals.

### Observability

- Tracing: `tracing` crate with OpenTelemetry layer (`opentelemetry-otlp` exporter)
- Metrics: `opentelemetry` SDK
- Error tracking: `sentry` (optional feature, on by default)
- System metrics: `sysinfo`

### Minecraft Protocol Specifics

- RSA key generation + AES-CFB8 encryption for login
- SHA-1 hash for Mojang session server login verification (uses the non-standard Minecraft variant — negative hashes are hex-encoded with a leading `-`)
- Cookie-based session authentication signed with HMAC-SHA2
- Proxy Protocol support (HAProxy v1/v2) via `proxy-header`
- NBT tags via `fastnbt`
