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

# Boot the official Minecraft client against the real binary (needs Xvfb, a JDK and disk)
xvfb-run -a cargo test -p passage --test client_conformance -- --ignored --nocapture
```

## Workspace Structure

The root `passage` binary plus 7 workspace member crates. The crates have clear layering:

| Crate                     | Role                                                                             |
|---------------------------|----------------------------------------------------------------------------------|
| `passage`                 | Binary entry point: wires adapters together, loads config, starts the server     |
| `passage-router`          | Core TCP listener, Minecraft protocol state machine, connection handling, crypto |
| `passage-core`            | Minecraft protocol toolkit.                                                      |
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

`Routes<Stat, Disc, Auth, Loca>` in `passage-router` is parameterized by adapter types. Each `Route` holds a hostname regex pattern and one instance of each adapter, allowing different auth/discovery logic per virtual hostname.

### Configuration

Loaded in priority order (highest to lowest):
1. Environment variables with `PASSAGE_` prefix
2. Auth secret file (optional)
3. Config file (`config/config`, optional)
4. Hardcoded defaults

Uses the `config` crate. See `passage/src/config.rs`.

### Protocol State Machine

Connections progress through states: `Handshake → Status | Login → Configuration → Transfer`. The configuration phase handles resource pack delivery with keep-alive packets sent on 16-second intervals.

### Packet versioning

Each packet in `passage-core/src/packet` declares `IDS`: its ID in every protocol version that
changed it, newest first. The router reads those thresholds to decide how many dispatch tables to
build, so adding a version to `passage-core/src/common/version.rs` is only meaningful if a packet
actually names it.

IDs and field layouts are transcribed by hand from the
[protocol documentation](https://minecraft.wiki/w/Java_Edition_protocol/Packets), which is the
cheapest thing to get right while implementing and the cheapest to re-check by eye. Nothing in the
Rust test suite can confirm them: the flow tests in `passage-router/src/router/flow.rs` replay whole
conversations at every supported version, but both ends share `Packet::IDS` and the same encoder, so
a wrong ID or a field gated at the wrong version makes them agree with each other and pass.

What does confirm them is `tests/client_conformance.rs`, which boots the *official Minecraft client*
against the shipped binary once per breakpoint and requires it to follow the transfer. It is the
only test that shares no code with what it is judging. `#[ignore]`d; nightly via
`.github/workflows/conformance.yml`. When a new Minecraft version lands, add it to `BREAKPOINTS`
there and to `versions` in `passage-core/src/common/version.rs`.

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
