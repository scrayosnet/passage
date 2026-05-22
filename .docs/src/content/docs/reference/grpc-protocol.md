---
title: gRPC Protocol Reference
description: Complete reference for Passage gRPC adapter protocol definitions.
---

Complete reference for the gRPC protocol used by Passage's custom adapters. All services are defined in the `scrayosnet.passage.adapter` package.

For implementation examples, see [Custom gRPC Adapters](/advanced/grpc-adapters/).

## Proto Files

```
passage-adapters/grpc/proto/adapter/
├── adapter.proto           # Common types
├── status.proto            # Status service
├── authentication.proto    # Authentication service
├── discovery.proto         # Discovery service
├── discovery_action.proto  # Discovery Action service
└── localization.proto      # Localization service
```

## Services Overview

| Service | RPC | Request | Response | Config `type` |
|---------|-----|---------|----------|---------------|
| `Status` | `GetStatus` | `StatusRequest` | `StatusResponse` | `grpc` (in `status`) |
| `Authentication` | `Authenticate` | `AuthenticationRequest` | `AuthenticationResponse` | `grpc` (in `authentication`) |
| `Discovery` | `GetTargets` | `TargetRequest` | `TargetsResponse` | `grpc_discovery` (in `discovery`) |
| `DiscoveryAction` | `Apply` | `ApplyRequest` | `ApplyResponse` | `grpc` (in `discovery.actions`) |
| `Localization` | `Localize` | `LocalizationRequest` | `LocalizationResponse` | `grpc` (in `localization`) |

---

## Common Types (`adapter.proto`)

These types are shared across all services.

### `Address`

```protobuf
message Address {
    string hostname = 1;
    uint32 port = 2;
}
```

| Field | Type | Description |
|-------|------|-------------|
| `hostname` | string | Hostname or IP address |
| `port` | uint32 | Port number |

---

### `Target`

Represents a backend Minecraft server.

```protobuf
message Target {
    string identifier = 1;
    Address address = 2;
    repeated MetaEntry meta = 3;
    uint32 priority = 4;
}
```

| Field | Type | Description |
|-------|------|-------------|
| `identifier` | string | Unique name for the server |
| `address` | Address | Network address |
| `meta` | repeated MetaEntry | Key-value metadata pairs |
| `priority` | uint32 | Priority for ordering (lower = higher priority) |

---

### `MetaEntry`

```protobuf
message MetaEntry {
    string key = 1;
    string value = 2;
}
```

| Field | Type | Description |
|-------|------|-------------|
| `key` | string | Metadata key (e.g., `"type"`, `"players"`) |
| `value` | string | Metadata value (always a string) |

---

### `ClientInfo`

Client connection information, passed to Discovery, DiscoveryAction, and Authentication services.

```protobuf
message ClientInfo {
    Address client_address = 1;
    Address server_address = 2;
    uint64 protocol_version = 3;
}
```

| Field | Type | Description |
|-------|------|-------------|
| `client_address` | Address | The connecting client's address |
| `server_address` | Address | The address the client connected to |
| `protocol_version` | uint64 | Minecraft protocol version number |

---

### `PlayerInfo`

Player identity information.

```protobuf
message PlayerInfo {
    string name = 1;
    string id = 2;
}
```

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Player's username |
| `id` | string | Player's UUID (with hyphens) |

---

### `Profile`

Minecraft player profile, used in authentication responses.

```protobuf
message Profile {
    string id = 1;
    string name = 2;
    repeated ProfileProperty properties = 3;
    repeated string profile_actions = 4;
}
```

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | Player UUID |
| `name` | string | Player username |
| `properties` | repeated ProfileProperty | Profile properties (e.g., textures) |
| `profile_actions` | repeated string | Pending moderation actions |

---

### `ProfileProperty`

```protobuf
message ProfileProperty {
    string name = 1;
    string value = 2;
    optional string signature = 3;
}
```

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Property name (e.g., `"textures"`) |
| `value` | string | Base64-encoded property value |
| `signature` | string (optional) | Base64-encoded Mojang signature |

---

## Status Service (`status.proto`)

Provides server list status information for Minecraft client pings.

```protobuf
service Status {
    rpc GetStatus(StatusRequest) returns (StatusResponse);
}
```

### `StatusRequest`

```protobuf
message StatusRequest {
    Address client_address = 1;
    Address server_address = 2;
    uint64 protocol = 3;
}
```

| Field | Type | Description |
|-------|------|-------------|
| `client_address` | Address | The client's network address |
| `server_address` | Address | The address the client connected to |
| `protocol` | uint64 | Client's Minecraft protocol version |

### `StatusResponse`

```protobuf
message StatusResponse {
    optional StatusData status = 1;
}
```

If `status` is null, the connection is rejected.

### `StatusData`

```protobuf
message StatusData {
    ProtocolVersion version = 1;
    optional Players players = 2;
    optional string description = 3;
    optional bytes favicon = 4;
    optional bool enforces_secure_chat = 5;
}
```

| Field | Type | Description |
|-------|------|-------------|
| `version` | ProtocolVersion | Version and protocol info |
| `players` | Players (optional) | Player count and samples |
| `description` | string (optional) | MOTD as JSON text component |
| `favicon` | bytes (optional) | 64x64 PNG image data |
| `enforces_secure_chat` | bool (optional) | Whether secure chat is enforced |

### `ProtocolVersion`

```protobuf
message ProtocolVersion {
    string name = 1;
    int32 protocol = 2;
}
```

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Display name in server list (e.g., `"My Network"`) |
| `protocol` | int32 | Protocol version number (e.g., `769` for 1.21.4) |

### `Players`

```protobuf
message Players {
    uint32 online = 1;
    uint32 max = 2;
    repeated PlayerEntry samples = 3;
}
```

### `PlayerEntry`

```protobuf
message PlayerEntry {
    string name = 1;
    string id = 2;
}
```

**Example response:**
```json
{
  "status": {
    "version": {"name": "My Network", "protocol": 769},
    "players": {
      "online": 42, "max": 100,
      "samples": [{"name": "Steve", "id": "069a79f4-44e9-4726-a5be-fca90e38aaf5"}]
    },
    "description": "{\"text\":\"Welcome!\",\"color\":\"gold\"}"
  }
}
```

---

## Authentication Service (`authentication.proto`)

Verifies player identity using custom logic.

```protobuf
service Authentication {
    rpc Authenticate(AuthenticationRequest) returns (AuthenticationResponse);
}
```

### `AuthenticationRequest`

```protobuf
message AuthenticationRequest {
    ClientInfo client = 1;
    PlayerInfo player = 2;
    bytes shared_secret = 3;
    bytes encoded_public = 4;
}
```

| Field | Type | Description |
|-------|------|-------------|
| `client` | ClientInfo | Client and server addresses, protocol version |
| `player` | PlayerInfo | Player name and UUID |
| `shared_secret` | bytes | The encrypted shared secret from the client |
| `encoded_public` | bytes | The encoded public key |

### `AuthenticationResponse`

```protobuf
message AuthenticationResponse {
    oneof reason {
        Profile profile = 1;
        string key = 2;
    }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `profile` | Profile | Accept: the player's verified profile |
| `key` | string | Reject: a localization key for the disconnect message |

Return exactly one of `profile` or `key`:
- **`profile`**: Allows the connection with the given identity
- **`key`**: Disconnects the player with the localized message for that key (e.g., `"disconnect_unauthenticated"`)

---

## Discovery Service (`discovery.proto`)

Discovers available backend servers.

```protobuf
service Discovery {
    rpc GetTargets(TargetRequest) returns (TargetsResponse);
}
```

### `TargetRequest`

```protobuf
message TargetRequest {
    ClientInfo client = 1;
}
```

| Field | Type | Description |
|-------|------|-------------|
| `client` | ClientInfo | Client address, server address, and protocol version |

### `TargetsResponse`

```protobuf
message TargetsResponse {
    repeated Target targets = 1;
}
```

Returns a list of available backend servers with metadata.

**Example request/response:**
```json
// Request
{"client": {"client_address": {"hostname": "192.168.1.100", "port": 54321}, "server_address": {"hostname": "mc.example.net", "port": 25565}, "protocol_version": 769}}

// Response
{"targets": [{"identifier": "hub-1", "address": {"hostname": "10.0.1.10", "port": 25565}, "meta": [{"key": "players", "value": "15"}]}]}
```

---

## Discovery Action Service (`discovery_action.proto`)

Transforms the target list in the actions pipeline.

```protobuf
service DiscoveryAction {
    rpc Apply(ApplyRequest) returns (ApplyResponse);
}
```

### `ApplyRequest`

```protobuf
message ApplyRequest {
    ClientInfo client = 1;
    PlayerInfo player = 2;
    repeated Target targets = 3;
}
```

| Field | Type | Description |
|-------|------|-------------|
| `client` | ClientInfo | Client connection information |
| `player` | PlayerInfo | Player name and UUID |
| `targets` | repeated Target | Current target list to process |

### `ApplyResponse`

```protobuf
message ApplyResponse {
    oneof reason {
        Targets targets = 1;
        string key = 2;
    }
}

message Targets {
    repeated Target targets = 1;
}
```

| Field | Type | Description |
|-------|------|-------------|
| `targets` | Targets | Accept: the modified target list |
| `key` | string | Reject: a localization key for the disconnect message |

Return exactly one of `targets` or `key`:
- **`targets`**: Returns the modified target list (can filter, reorder, or replace targets)
- **`key`**: Rejects the player with a localized disconnect message

---

## Localization Service (`localization.proto`)

Provides translated disconnect messages.

```protobuf
service Localization {
    rpc Localize(LocalizationRequest) returns (LocalizationResponse);
}
```

### `LocalizationRequest`

```protobuf
message LocalizationRequest {
    optional string locale = 1;
    string key = 2;
    map<string, string> params = 3;
}
```

| Field | Type | Description |
|-------|------|-------------|
| `locale` | string (optional) | Player's client locale (e.g., `"en"`, `"de"`) |
| `key` | string | Message key to localize (e.g., `"disconnect_timeout"`) |
| `params` | map | Substitution parameters |

### `LocalizationResponse`

```protobuf
message LocalizationResponse {
    string message = 1;
}
```

| Field | Type | Description |
|-------|------|-------------|
| `message` | string | The localized message as a Minecraft JSON text component |

---

## Testing with grpcurl

```bash
# Status
grpcurl -plaintext -import-path ./proto -proto adapter/status.proto \
  -d '{"client_address":{"hostname":"127.0.0.1","port":12345},"server_address":{"hostname":"localhost","port":25565},"protocol":769}' \
  localhost:50051 scrayosnet.passage.adapter.Status/GetStatus

# Authentication
grpcurl -plaintext -import-path ./proto -proto adapter/authentication.proto \
  -d '{"client":{"client_address":{"hostname":"127.0.0.1","port":12345},"protocol_version":769},"player":{"name":"Steve","id":"069a79f4-44e9-4726-a5be-fca90e38aaf5"}}' \
  localhost:50051 scrayosnet.passage.adapter.Authentication/Authenticate

# Discovery
grpcurl -plaintext -import-path ./proto -proto adapter/discovery.proto \
  -d '{"client":{"client_address":{"hostname":"127.0.0.1","port":12345},"protocol_version":769}}' \
  localhost:50051 scrayosnet.passage.adapter.Discovery/GetTargets

# Discovery Action
grpcurl -plaintext -import-path ./proto -proto adapter/discovery_action.proto \
  -d '{"client":{"client_address":{"hostname":"127.0.0.1","port":12345}},"player":{"name":"Steve","id":"069a79f4-44e9-4726-a5be-fca90e38aaf5"},"targets":[{"identifier":"hub-1","address":{"hostname":"10.0.1.10","port":25565}}]}' \
  localhost:50051 scrayosnet.passage.adapter.DiscoveryAction/Apply

# Localization
grpcurl -plaintext -import-path ./proto -proto adapter/localization.proto \
  -d '{"locale":"en","key":"disconnect_timeout"}' \
  localhost:50051 scrayosnet.passage.adapter.Localization/Localize
```
