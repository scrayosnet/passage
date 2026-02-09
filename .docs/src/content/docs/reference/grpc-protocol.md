---
title: gRPC Protocol Reference
description: Complete reference for Passage gRPC adapter protocol definitions.
---

This page provides a complete reference for the gRPC protocol used by Passage's custom adapters. Use this reference when implementing your own gRPC adapters in any language.

## Overview

Passage defines three gRPC services for extending functionality:

- **Status Service** - Provides server list status information (MOTD, player count, favicon)
- **Discovery Service** - Discovers available backend servers
- **Strategy Service** - Selects which server to route each player to

All services are defined in the `scrayosnet.passage.adapter` package.

## Proto Files

Proto definitions are located in the Passage repository:

```
proto/adapter/
├── adapter.proto     # Common types (Target, Address, MetaEntry)
├── status.proto      # Status service definition
├── discovery.proto   # Discovery service definition
└── strategy.proto    # Strategy service definition
```

## Common Types

### `Address`

Represents a network socket address.

```protobuf
message Address {
    string hostname = 1;
    uint32 port = 2;
}
```

**Fields:**
- `hostname` (string): Hostname or IP address (e.g., `"10.0.1.10"`, `"minecraft.example.com"`)
- `port` (uint32): Port number (e.g., `25565`)

**Example:**
```json
{
  "hostname": "10.0.1.10",
  "port": 25565
}
```

---

### `Target`

Represents a backend Minecraft server.

```protobuf
message Target {
    string identifier = 1;
    Address address = 2;
    repeated MetaEntry meta = 3;
}
```

**Fields:**
- `identifier` (string): Unique identifier for this server (e.g., `"hub-1"`, `"survival-east-2"`)
- `address` (Address): Network address of the server
- `meta` (repeated MetaEntry): Optional metadata key-value pairs

**Example:**
```json
{
  "identifier": "hub-1",
  "address": {
    "hostname": "10.0.1.10",
    "port": 25565
  },
  "meta": [
    {"key": "type", "value": "hub"},
    {"key": "region", "value": "us-east"},
    {"key": "players", "value": "15"}
  ]
}
```

---

### `MetaEntry`

Key-value pair for storing server metadata.

```protobuf
message MetaEntry {
    string key = 1;
    string value = 2;
}
```

**Fields:**
- `key` (string): Metadata key (e.g., `"type"`, `"region"`, `"players"`)
- `value` (string): Metadata value (always string, even for numbers)

**Common Metadata Keys:**
- `type`: Server type (e.g., `"hub"`, `"survival"`, `"minigame"`)
- `region`: Geographic region (e.g., `"us-east"`, `"eu-west"`)
- `players`: Current player count (e.g., `"15"`)
- `max_players`: Maximum players (e.g., `"100"`)
- `version`: Minecraft version (e.g., `"1.21.4"`)

---

## Status Service

Provides server list status information for Minecraft client pings.

### Service Definition

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

**Fields:**
- `client_address` (Address): The client's network address
- `server_address` (Address): The address the client connected to
- `protocol` (uint64): Minecraft protocol version number the client is using

**Protocol Version Examples:**
- `769` - Minecraft 1.21.4
- `767` - Minecraft 1.21.1
- `766` - Minecraft 1.20.5
- `763` - Minecraft 1.20.1

**Example Request:**
```json
{
  "client_address": {
    "hostname": "192.168.1.100",
    "port": 54321
  },
  "server_address": {
    "hostname": "play.example.com",
    "port": 25565
  },
  "protocol": 769
}
```

---

### `StatusResponse`

```protobuf
message StatusResponse {
    optional StatusData status = 1;
}
```

**Fields:**
- `status` (StatusData, optional): Server status data. If null, connection is rejected.

---

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

**Fields:**
- `version` (ProtocolVersion, required): Version information
- `players` (Players, optional): Player count information
- `description` (string, optional): MOTD as JSON text component
- `favicon` (bytes, optional): 64x64 PNG image data
- `enforces_secure_chat` (bool, optional): Whether secure chat is enforced (1.19+)

**Example:**
```json
{
  "status": {
    "version": {
      "name": "My Minecraft Network",
      "protocol": 769
    },
    "players": {
      "online": 42,
      "max": 100,
      "samples": [
        {"name": "Steve", "id": "069a79f4-44e9-4726-a5be-fca90e38aaf5"},
        {"name": "Alex", "id": "ec561538-f3fd-461d-aff5-086b22154bce"}
      ]
    },
    "description": "{\"text\":\"Welcome to our server!\",\"color\":\"gold\"}",
    "enforces_secure_chat": true
  }
}
```

---

### `ProtocolVersion`

```protobuf
message ProtocolVersion {
    string name = 1;
    int32 protocol = 2;
}
```

**Fields:**
- `name` (string): Display name shown in server list
- `protocol` (int32): Minecraft protocol version number

---

### `Players`

```protobuf
message Players {
    uint32 online = 1;
    uint32 max = 2;
    repeated PlayerEntry samples = 3;
}
```

**Fields:**
- `online` (uint32): Current number of online players
- `max` (uint32): Maximum player capacity
- `samples` (repeated PlayerEntry): Sample player entries shown on hover

---

### `PlayerEntry`

```protobuf
message PlayerEntry {
    string name = 1;
    string id = 2;
}
```

**Fields:**
- `name` (string): Player display name
- `id` (string): Player UUID (with hyphens)

---

## Discovery Service

Discovers available backend Minecraft servers.

### Service Definition

```protobuf
service Discovery {
    rpc GetTargets(TargetRequest) returns (TargetsResponse);
}
```

### `TargetRequest`

```protobuf
message TargetRequest {
    Address client_address = 1;
    Address server_address = 2;
    uint64 protocol = 3;
    string username = 4;
    string user_id = 5;
}
```

**Fields:**
- `client_address` (Address): The client's network address
- `server_address` (Address): The address the client connected to
- `protocol` (uint64): Minecraft protocol version
- `username` (string): Player's username (e.g., `"Steve"`)
- `user_id` (string): Player's UUID (with hyphens, e.g., `"069a79f4-44e9-4726-a5be-fca90e38aaf5"`)

**Example Request:**
```json
{
  "client_address": {
    "hostname": "192.168.1.100",
    "port": 54321
  },
  "server_address": {
    "hostname": "play.example.com",
    "port": 25565
  },
  "protocol": 769,
  "username": "Steve",
  "user_id": "069a79f4-44e9-4726-a5be-fca90e38aaf5"
}
```

---

### `TargetsResponse`

```protobuf
message TargetsResponse {
    repeated Target targets = 1;
}
```

**Fields:**
- `targets` (repeated Target): List of available backend servers

**Example Response:**
```json
{
  "targets": [
    {
      "identifier": "hub-1",
      "address": {"hostname": "10.0.1.10", "port": 25565},
      "meta": [
        {"key": "type", "value": "hub"},
        {"key": "players", "value": "15"}
      ]
    },
    {
      "identifier": "survival-1",
      "address": {"hostname": "10.0.2.10", "port": 25565},
      "meta": [
        {"key": "type", "value": "survival"},
        {"key": "players", "value": "8"}
      ]
    }
  ]
}
```

---

## Strategy Service

Selects which backend server to route a player to.

### Service Definition

```protobuf
service Strategy {
    rpc SelectTarget(SelectRequest) returns (SelectResponse);
}
```

### `SelectRequest`

```protobuf
message SelectRequest {
    Address client_address = 1;
    Address server_address = 2;
    uint64 protocol = 3;
    string username = 4;
    string user_id = 5;
    repeated Target targets = 6;
}
```

**Fields:**
- `client_address` (Address): The client's network address
- `server_address` (Address): The address the client connected to
- `protocol` (uint64): Minecraft protocol version
- `username` (string): Player's username
- `user_id` (string): Player's UUID (with hyphens)
- `targets` (repeated Target): Available servers to choose from (from Discovery)

**Example Request:**
```json
{
  "client_address": {
    "hostname": "192.168.1.100",
    "port": 54321
  },
  "server_address": {
    "hostname": "play.example.com",
    "port": 25565
  },
  "protocol": 769,
  "username": "Steve",
  "user_id": "069a79f4-44e9-4726-a5be-fca90e38aaf5",
  "targets": [
    {
      "identifier": "hub-1",
      "address": {"hostname": "10.0.1.10", "port": 25565},
      "meta": [{"key": "players", "value": "15"}]
    },
    {
      "identifier": "hub-2",
      "address": {"hostname": "10.0.1.11", "port": 25565},
      "meta": [{"key": "players", "value": "8"}]
    }
  ]
}
```

---

### `SelectResponse`

```protobuf
message SelectResponse {
    optional Target target = 1;
}
```

**Fields:**
- `target` (Target, optional): Selected server. If null, connection is rejected.

**Example Response:**
```json
{
  "target": {
    "identifier": "hub-2",
    "address": {"hostname": "10.0.1.11", "port": 25565},
    "meta": [{"key": "players", "value": "8"}]
  }
}
```

---

## Implementation Guide

### Code Generation

Generate gRPC code for your language:

#### Go
```bash
protoc --go_out=. --go_opt=paths=source_relative \
    --go-grpc_out=. --go-grpc_opt=paths=source_relative \
    proto/adapter/*.proto
```

#### Python
```bash
python -m grpc_tools.protoc \
    -I. --python_out=. --grpc_python_out=. \
    proto/adapter/*.proto
```

#### Java
```bash
protoc --java_out=src/main/java \
    --grpc-java_out=src/main/java \
    proto/adapter/*.proto
```

#### Node.js
```bash
grpc_tools_node_protoc \
    --js_out=import_style=commonjs,binary:. \
    --grpc_out=grpc_js:. \
    proto/adapter/*.proto
```

#### Rust
Add to `build.rs`:
```rust
fn main() {
    tonic_build::configure()
        .compile(&["proto/adapter/status.proto"], &["proto"])
        .unwrap();
}
```

---

### Server Implementation

Implement the gRPC service interface:

**Go Example:**
```go
type statusServer struct {
    pb.UnimplementedStatusServer
}

func (s *statusServer) GetStatus(ctx context.Context, req *pb.StatusRequest) (*pb.StatusResponse, error) {
    return &pb.StatusResponse{
        Status: &pb.StatusData{
            Version: &pb.ProtocolVersion{
                Name:     "My Server",
                Protocol: int32(req.Protocol),
            },
            Players: &pb.Players{
                Online: 10,
                Max:    100,
            },
            Description: `{"text":"Welcome!"}`,
        },
    }, nil
}
```

**Python Example:**
```python
class StatusService(status_pb2_grpc.StatusServicer):
    def GetStatus(self, request, context):
        return status_pb2.StatusResponse(
            status=status_pb2.StatusData(
                version=status_pb2.ProtocolVersion(
                    name="My Server",
                    protocol=request.protocol
                ),
                players=status_pb2.Players(
                    online=10,
                    max=100
                ),
                description='{"text":"Welcome!"}'
            )
        )
```

---

### Testing with grpcurl

Test your gRPC services using `grpcurl`:

```bash
# Install grpcurl
brew install grpcurl  # macOS
# or
go install github.com/fullstorydev/grpcurl/cmd/grpcurl@latest

# Test Status service
grpcurl -plaintext -d '{
  "client_address": {"hostname": "127.0.0.1", "port": 12345},
  "server_address": {"hostname": "localhost", "port": 25565},
  "protocol": 769
}' localhost:3030 scrayosnet.passage.adapter.Status/GetStatus

# Test Discovery service
grpcurl -plaintext -d '{
  "username": "Steve",
  "user_id": "069a79f4-44e9-4726-a5be-fca90e38aaf5",
  "protocol": 769
}' localhost:3030 scrayosnet.passage.adapter.Discovery/GetTargets

# Test Strategy service
grpcurl -plaintext -d '{
  "username": "Steve",
  "user_id": "069a79f4-44e9-4726-a5be-fca90e38aaf5",
  "targets": [
    {
      "identifier": "hub-1",
      "address": {"hostname": "10.0.1.10", "port": 25565}
    }
  ]
}' localhost:3030 scrayosnet.passage.adapter.Strategy/SelectTarget
```

---

## Error Handling

### gRPC Status Codes

Use appropriate gRPC status codes for errors:

- `OK` - Success
- `INVALID_ARGUMENT` - Invalid request data
- `NOT_FOUND` - Resource not found
- `UNAVAILABLE` - Service temporarily unavailable
- `INTERNAL` - Internal server error

**Go Example:**
```go
import "google.golang.org/grpc/codes"
import "google.golang.org/grpc/status"

func (s *strategyServer) SelectTarget(ctx context.Context, req *pb.SelectRequest) (*pb.SelectResponse, error) {
    if req.Username == "" {
        return nil, status.Error(codes.InvalidArgument, "username is required")
    }
    // ... implementation
}
```

**Python Example:**
```python
import grpc

def SelectTarget(self, request, context):
    if not request.username:
        context.set_code(grpc.StatusCode.INVALID_ARGUMENT)
        context.set_details('username is required')
        return strategy_pb2.SelectResponse()
    # ... implementation
```

---

## Best Practices

### Performance
- **Keep response times under 50ms** - Slow adapters delay player connections
- **Use connection pooling** - Reuse database/API connections
- **Implement caching** - Cache expensive operations
- **Return quickly** - Avoid blocking operations

### Reliability
- **Always return a response** - Never timeout or hang
- **Handle errors gracefully** - Return sensible defaults on error
- **Log all requests** - Aid debugging and monitoring
- **Implement health checks** - Use gRPC health checking protocol

### Security
- **Validate all inputs** - Don't trust request data
- **Use TLS in production** - Configure `https://` endpoints
- **Authenticate requests** - Use gRPC authentication if needed
- **Rate limit** - Protect from abuse
