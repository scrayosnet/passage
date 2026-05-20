---
title: Authentication Adapter
description: Configure how Passage authenticates connecting players.
sidebar:
    order: 3
---

The authentication adapter verifies player identity before routing them to a backend server. Each [route](/reference/configuration/#routes) can use a different authentication method.

## Mojang Adapter (Default)

Validates players against the official Mojang/Microsoft session servers. This is the standard Minecraft authentication flow and the recommended choice for production deployments.

```yaml
# config/config.yaml
routes:
- hostname: "mc.example.net"
  authentication:
    type: mojang
    server_id: ""
```

### How It Works

1. Passage sends an **Encryption Request** with a generated RSA public key
2. The client authenticates with Mojang's session servers using the shared secret
3. Passage verifies the session via `GET https://sessionserver.mojang.com/session/minecraft/hasJoined`
4. The player's profile (UUID, name, skin) is received

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `server_id` | string | `""` | Server ID passed to Mojang. Usually left empty. |

:::tip[When to Use]
Use `mojang` authentication for all production networks. It ensures only legitimate Minecraft accounts can connect.
:::

---

## Disabled Adapter

Skips authentication entirely. The connection is not encrypted.

```yaml
routes:
- hostname: "test.example.net"
  authentication:
    type: disabled
```

:::caution[Security Warning]
Never use `disabled` authentication in production. Without authentication, anyone can connect with any username and UUID. This is intended only for local development and testing.
:::

---

## Fixed Adapter

Uses a hardcoded player profile for all connections. Useful for testing and development environments where you want a consistent identity.

```yaml
routes:
- hostname: "dev.example.net"
  authentication:
    type: fixed
    profile:
      id: "069a79f4-44e9-4726-a5be-fca90e38aaf5"
      name: "Steve"
      properties:
      - name: "textures"
        value: "base64_encoded_texture_data"
        signature: "base64_encoded_signature"
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `profile` | object (optional) | `null` | The fixed Minecraft profile to use for all connections. |
| `profile.id` | string (UUID) | *required* | The unique profile UUID. |
| `profile.name` | string | *required* | The player's username. |
| `profile.properties` | array | `[]` | Profile properties (e.g., textures). |
| `profile.profileActions` | array | `[]` | Pending moderation actions. |

---

## gRPC Adapter

Delegates authentication to an external gRPC service for custom validation logic. This allows you to implement custom authentication systems, additional verification steps, or integration with your own account systems.

```yaml
routes:
- hostname: "mc.example.net"
  authentication:
    type: grpc
    address: "http://auth-service:50051"
```

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `address` | string | `""` | The gRPC service endpoint URL. |

### gRPC Service Definition

The service must implement the `Authentication` service from `authentication.proto`:

```protobuf
service Authentication {
    rpc Authenticate(AuthenticationRequest) returns (AuthenticationResponse);
}
```

The `AuthenticationRequest` includes:
- `client` (`ClientInfo`): Client and server address, protocol version
- `player` (`PlayerInfo`): Player name and UUID
- `shared_secret` (bytes): The encrypted shared secret
- `encoded_public` (bytes): The encoded public key

The `AuthenticationResponse` uses a `oneof` field:
- Return a `Profile` to allow the connection with a specific identity
- Return a `key` (string) to reject the connection with a localization key

See the [gRPC Protocol Reference](/reference/grpc-protocol/) for full message definitions and the [Custom gRPC Adapters](/advanced/grpc-adapters/) guide for implementation examples.

---

## Choosing an Authentication Method

| Use Case | Recommended Type |
|----------|-----------------|
| Production network | `mojang` |
| Local development | `disabled` or `fixed` |
| Testing with consistent identity | `fixed` |
| Custom account system | `grpc` |
| Whitelisted/closed beta | `grpc` (with your own verification) |
