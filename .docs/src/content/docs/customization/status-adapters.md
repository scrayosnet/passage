---
title: Status Adapters
description: Configure how Passage responds to server list pings.
---

Status adapters provide the server information shown in the Minecraft multiplayer server list (MOTD, player count, favicon, etc.).

## Overview

When a player refreshes their server list, Minecraft sends a status request to Passage. The Status Adapter determines what information is returned.

## Available Adapters

### Fixed Status Adapter

Returns static, configured server status information.

**Use when:** You want a simple, static server list entry.

#### Configuration

```toml
[status]
adapter = "fixed"

[status.fixed]
name = "My Minecraft Network"
description = "\"Welcome to my awesome server!\""
favicon = "data:image/png;base64,iVBORw0KGgo..."
enforces_secure_chat = true
preferred_version = 769  # Minecraft 1.21
min_version = 766        # Minecraft 1.20.5
max_version = 1000       # Future versions
```

#### Parameters

- **`name`** (string, required): Server name shown in the version field
- **`description`** (string, optional): MOTD shown to players (JSON text format, must be escaped)
- **`favicon`** (string, optional): Base64-encoded PNG image (64x64 pixels)
- **`enforces_secure_chat`** (boolean, optional): Whether chat signing is enforced
- **`preferred_version`** (number, required): Protocol version to advertise
- **`min_version`** (number, required): Minimum supported protocol version
- **`max_version`** (number, required): Maximum supported protocol version

#### Description Format

The description must be valid Minecraft JSON text format, escaped for TOML:

```toml
# Simple text
description = "\"Welcome to my server!\""

# Formatted text
description = "{\"text\":\"Welcome!\",\"color\":\"gold\",\"bold\":true}"

# Multi-line text (using TOML multi-line strings)
description = '''
{
  "extra": [
    {"text": "Welcome to ", "color": "gray"},
    {"text": "My Network", "color": "gold", "bold": true}
  ]
}
'''
```

#### Favicon Format

Convert a 64x64 PNG image to base64:

```bash
# Linux/macOS
base64 -w 0 server-icon.png > favicon.txt

# Then in config
favicon = "data:image/png;base64,iVBORw0KGgo..."
```

Or use online tools like [base64-image.de](https://www.base64-image.de/).

#### Protocol Versions

The adapter adjusts the reported protocol version based on the client:

- If client protocol is between `min_version` and `max_version`: Returns client's protocol
- Otherwise: Returns `preferred_version`

Common protocol versions:
- `769`: Minecraft 1.21
- `767`: Minecraft 1.21.1
- `766`: Minecraft 1.20.5/1.20.6
- See [wiki.vg Protocol Version Numbers](https://wiki.vg/Protocol_version_numbers) for full list

#### Example

```toml
[status]
adapter = "fixed"

[status.fixed]
name = "§6§lAwesome Network"
description = '''
{
  "extra": [
    {"text": "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n", "color": "gold"},
    {"text": "     Welcome to ", "color": "gray"},
    {"text": "Awesome Network", "color": "gold", "bold": true},
    {"text": "\n\n     ", "color": "white"},
    {"text": "⚔ ", "color": "red"},
    {"text": "SkyWars  ", "color": "gray"},
    {"text": "⛏ ", "color": "aqua"},
    {"text": "Survival  ", "color": "gray"},
    {"text": "🏆 ", "color": "yellow"},
    {"text": "BedWars", "color": "gray"},
    {"text": "\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━", "color": "gold"}
  ]
}
'''
favicon = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAA..."
enforces_secure_chat = true
preferred_version = 769
min_version = 766
max_version = 1000
```

---

### HTTP Status Adapter

Queries a HTTP endpoint for dynamic server status.

**Use when:** You want to dynamically generate status from an external service.

#### Configuration

```toml
[status]
adapter = "http"

[status.http]
address = "https://api.example.com/minecraft/status"
cache_duration = 5  # Cache for 5 seconds
```

#### Parameters

- **`address`** (string, required): URL of the HTTP endpoint
- **`cache_duration`** (number, required): How long to cache the response (seconds)

#### HTTP Endpoint Requirements

Your endpoint must:
1. Respond to GET requests
2. Return HTTP 200 status code
3. Return JSON matching the ServerStatus format

Response format:

```json
{
  "version": {
    "name": "Passage Network",
    "protocol": 769
  },
  "players": {
    "online": 42,
    "max": 100,
    "sample": [
      {"name": "Steve", "id": "8667ba71-b85a-4004-af54-457a9734eed7"},
      {"name": "Alex", "id": "ec561538-f3fd-461d-aff5-086b22154bce"}
    ]
  },
  "description": {
    "text": "Welcome to the network!"
  },
  "favicon": "data:image/png;base64,iVBORw0KGgo...",
  "enforcesSecureChat": true
}
```

All fields are optional except `version`.

#### Caching

The HTTP adapter caches responses to avoid overwhelming your backend:

- Responses are cached for `cache_duration` seconds
- All concurrent status requests use the cached value
- Cache refreshes automatically in the background
- If the HTTP request fails, the old cached value is used

#### Example Implementation (Python/Flask)

```python
from flask import Flask, jsonify

app = Flask(__name__)

@app.route('/minecraft/status')
def get_status():
    return jsonify({
        "version": {
            "name": "My Network",
            "protocol": 769
        },
        "players": {
            "online": get_online_count(),
            "max": 100
        },
        "description": {
            "text": "Welcome!",
            "color": "gold"
        },
        "enforcesSecureChat": True
    })

if __name__ == '__main__':
    app.run(host='0.0.0.0', port=8080)
```

---

### gRPC Status Adapter

Queries a gRPC service for dynamic server status.

**Use when:** You want high-performance dynamic status with custom logic.

#### Configuration

```toml
[status]
adapter = "grpc"

[status.grpc]
address = "http://status-service:3030"
```

#### Parameters

- **`address`** (string, required): Address of the gRPC service (with scheme: `http://` or `https://`)

#### gRPC Service Definition

Your service must implement the `Status` service from the proto definition:

```protobuf
syntax = "proto3";

service Status {
    rpc GetStatus(StatusRequest) returns (StatusResponse);
}

message StatusRequest {
    Address client_address = 1;
    Address server_address = 2;
    uint64 protocol = 3;
}

message StatusResponse {
    optional StatusData status = 1;
}

message StatusData {
    ProtocolVersion version = 1;
    optional Players players = 2;
    optional string description = 3;
    optional bytes favicon = 4;
    optional bool enforces_secure_chat = 5;
}
```

See the full proto definitions in `/proto/adapter/` of the Passage repository.

#### Request Context

Your service receives:
- **client_address**: IP and port of the connecting client
- **server_address**: The server address the client is connecting to
- **protocol**: The protocol version the client is using

Use this to customize status per-client, per-domain, or per-version.

#### Example Implementation (Go)

```go
package main

import (
    "context"
    pb "github.com/scrayosnet/passage/proto/adapter"
)

type statusServer struct {
    pb.UnimplementedStatusServer
}

func (s *statusServer) GetStatus(ctx context.Context, req *pb.StatusRequest) (*pb.StatusResponse, error) {
    return &pb.StatusResponse{
        Status: &pb.StatusData{
            Version: &pb.ProtocolVersion{
                Name:     "My Network",
                Protocol: 769,
            },
            Players: &pb.Players{
                Online: 42,
                Max:    100,
            },
            Description: "{\"text\":\"Welcome!\"}",
            EnforcesSecureChat: true,
        },
    }, nil
}
```

See [Custom gRPC Adapters](/advanced/custom-grpc-adapters/) for a complete implementation guide.

## Choosing an Adapter

| Adapter | Performance | Flexibility | Complexity | Use Case |
|---------|-------------|-------------|------------|----------|
| **Fixed** | Fastest | Low | None | Static networks |
| **HTTP** | Medium | Medium | Low | Simple dynamic status |
| **gRPC** | Fastest | Highest | Medium | Complex custom logic |

## Best Practices

### Performance
- Use **Fixed** for static information
- Set appropriate `cache_duration` for HTTP adapter (3-10 seconds)
- Keep gRPC services fast (<10ms response time)

### Content
- Keep MOTD concise (2-3 lines max)
- Use favicon for branding
- Show accurate player counts when possible
- Advertise supported protocol versions correctly

### Security
- Don't expose sensitive information in status
- Rate limit your HTTP/gRPC services
- Validate and sanitize any dynamic content

## Troubleshooting

### Status not showing in server list

1. Check Passage is running: `netstat -tuln | grep 25565`
2. Test with Minecraft client
3. Check adapter configuration
4. Enable debug logging: `RUST_LOG=debug passage`

### HTTP adapter timing out

1. Verify the URL is accessible from Passage's network
2. Check HTTP service is running and responding
3. Reduce `cache_duration` if service is slow
4. Add timeout/retry logic to your HTTP service

### gRPC adapter connection failed

1. Verify gRPC service address includes scheme (`http://` or `https://`)
2. Check gRPC service is running: `grpcurl -plaintext localhost:3030 list`
3. Verify proto definitions match Passage's expectations
4. Check network connectivity between Passage and gRPC service

## Next Steps

- Configure [Target Discovery Adapters](/customization/target-discovery-adapters/)
- Learn about [Target Strategy Adapters](/customization/target-strategy-adapters/)
- Implement [Custom gRPC Adapters](/advanced/custom-grpc-adapters/)
