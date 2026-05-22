---
title: Status Adapter
description: Configure how Passage responds to server list pings.
sidebar:
    order: 1
---

The status adapter controls what players see in the Minecraft server list -- the MOTD, player count, version, and favicon. Each [route](/reference/configuration/#routes) can use a different status adapter.

## Fixed Adapter (Default)

Returns a static, preconfigured status response.

```yaml
routes:
- hostname: "mc.example.net"
  status:
    type: fixed
    name: "My Network"
    description: "\"Welcome to our server!\""
    favicon: "data:image/png;base64,..."
    enforces_secure_chat: true
    preferred_version: 769
    min_version: 766
    max_version: 1000
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | string | `"Passage"` | Server name displayed in the version field |
| `description` | string | `"\"Minecraft Server Transfer Router\""` | MOTD as a JSON text component |
| `favicon` | string | *(built-in icon)* | Base64-encoded 64x64 PNG (`data:image/png;base64,...`) |
| `enforces_secure_chat` | bool | `true` | Whether the server enforces secure chat |
| `preferred_version` | integer | `769` | Protocol version shown in the server list |
| `min_version` | integer | `0` | Minimum accepted protocol version |
| `max_version` | integer | `1000` | Maximum accepted protocol version |

### MOTD Formatting

The `description` field uses Minecraft's JSON text component format:

```yaml
# Simple text
description: "\"Welcome!\""

# Colored text
description: '{"text":"Welcome!","color":"gold"}'

# Multi-line with formatting
description: '{"text":"","extra":[{"text":"My Network\n","color":"gold","bold":true},{"text":"Join now!","color":"gray"}]}'
```

### Favicon

Generate a favicon from a 64x64 PNG image:

```bash
echo -n "data:image/png;base64,$(base64 -w 0 server-icon.png)"
```

### Protocol Versions

Common Minecraft protocol versions:

| Version | Protocol |
|---------|----------|
| 1.21.4 | `769` |
| 1.21.2 | `768` |
| 1.21 | `767` |
| 1.20.5 | `766` |

See [wiki.vg](https://wiki.vg/Protocol_version_numbers) for a complete list.

---

## HTTP Adapter

Fetches status from an HTTP endpoint. Responses are cached to avoid hitting the endpoint on every server list ping.

```yaml
routes:
- hostname: "mc.example.net"
  status:
    type: http
    address: "https://api.example.com/minecraft/status"
    cache_duration: 60
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `address` | string | `"http://localhost:8080"` | HTTP endpoint URL |
| `cache_duration` | integer | `60` | Cache duration in seconds |

### Endpoint Requirements

The endpoint must accept a GET request and return JSON matching the Minecraft status response format:

```json
{
  "version": {"name": "My Network", "protocol": 769},
  "players": {"online": 42, "max": 100, "sample": []},
  "description": {"text": "Welcome!"},
  "favicon": "data:image/png;base64,...",
  "enforcesSecureChat": true
}
```

:::tip
The HTTP adapter is great for status pages that aggregate player counts across your backend servers or show dynamic information. Set `cache_duration` to at least 10-30 seconds to avoid overloading your status endpoint.
:::

---

## gRPC Adapter

Delegates status generation to an external gRPC service for full control over the response.

```yaml
routes:
- hostname: "mc.example.net"
  status:
    type: grpc
    address: "http://status-service:50051"
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `address` | string | `""` | The gRPC service endpoint URL |

### gRPC Service Definition

The service must implement the `Status` service from `status.proto`:

```protobuf
service Status {
    rpc GetStatus(StatusRequest) returns (StatusResponse);
}
```

The `StatusRequest` includes:
- `client_address` (Address): The client's network address
- `server_address` (Address): The address the client connected to
- `protocol` (uint64): The client's Minecraft protocol version

See the [gRPC Protocol Reference](/reference/grpc-protocol/) for full message definitions and the [Custom gRPC Adapters](/advanced/grpc-adapters/) guide for implementation examples.

---

## Choosing a Status Adapter

| Use Case | Recommended Type |
|----------|--------------------|
| Static server with fixed MOTD | `fixed` |
| Dynamic player count or rotating MOTD | `http` |
| Complex logic (per-player status, A/B testing) | `grpc` |
