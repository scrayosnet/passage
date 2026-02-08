---
title: Status Adapters
description: Configure how Passage responds to server list pings.
---

Status adapters provide server information for the Minecraft multiplayer server list (MOTD, player count, favicon).

## Fixed Adapter

Static configuration from TOML.

```toml
[status]
adapter = "fixed"

[status.fixed]
name = "My Network"
description = "\"Welcome!\""
favicon = "data:image/png;base64,..."
enforces_secure_chat = true
preferred_version = 769  # Minecraft 1.21
min_version = 766        # 1.20.5+
max_version = 1000
```

**Description format:** JSON text, escaped for TOML. Use `\"text\"` for simple strings, or full JSON objects for formatting.

**Favicon:** Base64-encoded 64x64 PNG. Generate with: `base64 -w 0 server-icon.png`

**Protocol versions:** See [wiki.vg](https://wiki.vg/Protocol_version_numbers) for version numbers.

## HTTP Adapter

Queries an HTTP endpoint for dynamic status. Caches responses for `cache_duration` seconds.

```toml
[status]
adapter = "http"

[status.http]
address = "https://api.example.com/minecraft/status"
cache_duration = 5
```

**Endpoint requirements:** GET request, return JSON:

```json
{
  "version": {"name": "My Network", "protocol": 769},
  "players": {"online": 42, "max": 100},
  "description": {"text": "Welcome!"},
  "favicon": "data:image/png;base64,...",
  "enforcesSecureChat": true
}
```

## gRPC Adapter

Custom gRPC service for full control.

```toml
[status]
adapter = "grpc"

[status.grpc]
address = "http://status-service:3030"
```

See [Custom gRPC Adapters](/advanced/custom-grpc-adapters/) for implementation details and proto definitions.
