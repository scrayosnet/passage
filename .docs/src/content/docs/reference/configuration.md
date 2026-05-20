---
title: Configuration Reference
description: Complete field-by-field reference for all Passage configuration options.
---

This page provides a comprehensive reference for every configuration field in Passage. For a beginner-friendly introduction, see [Configuration Basics](/setup/configuration-basics/).

:::tip[JSON Schema Available]
Passage provides a [JSON Schema](https://github.com/scrayosnet/passage/blob/main/config/schema.json) for configuration validation. Use it with your editor for autocompletion and validation:

```yaml
# yaml-language-server: $schema=./schema.json
```
:::

## Configuration Structure

Passage uses a layered configuration system supporting YAML, TOML, and JSON formats. The overall structure:

```yaml
# Core settings
address: "0.0.0.0:25565"
timeout: 120
max_packet_length: 10000
auth_cookie_expiry: 21600
auth_secret: "your-secret"

# Optional global features
sentry: { ... }
otel: { ... }
rate_limiter: { ... }
proxy_protocol: { ... }

# Routes (per-hostname adapter configuration)
routes:
- hostname: "mc.example.net"
  status: { type: fixed, ... }
  authentication: { type: mojang, ... }
  discovery: { type: dns_discovery, ..., actions: [...] }
  localization: { type: fixed, ... }
```

---

## Core Settings

### `address`

| | |
|---|---|
| **Type** | string (socket address) |
| **Default** | `"0.0.0.0:25565"` |
| **Environment** | `PASSAGE_ADDRESS` |

The network address and port that Passage binds to for incoming Minecraft client connections.

```yaml
# Listen on all interfaces, standard Minecraft port
address: "0.0.0.0:25565"

# Listen on specific interface
address: "192.168.1.100:25565"

# Use custom port
address: "0.0.0.0:25566"
```

---

### `timeout`

| | |
|---|---|
| **Type** | integer (seconds) |
| **Default** | `120` |
| **Environment** | `PASSAGE_TIMEOUT` |

Maximum time in seconds to wait for client responses during the connection flow. If the client does not respond within this time, the connection is dropped with a timeout message.

```yaml
timeout: 120  # 2 minutes (default)
timeout: 60   # shorter for high-performance scenarios
timeout: 300  # longer for slow connections
```

---

### `max_packet_length`

| | |
|---|---|
| **Type** | integer (bytes) |
| **Default** | `10000` |
| **Environment** | `PASSAGE_MAX_PACKET_LENGTH` |

The maximum packet size in bytes that Passage will accept. Packets exceeding this size are rejected. The default of 10,000 bytes is sufficient for normal Minecraft handshake and login packets.

```yaml
max_packet_length: 10000
```

---

### `auth_cookie_expiry`

| | |
|---|---|
| **Type** | integer (seconds) |
| **Default** | `21600` (6 hours) |
| **Environment** | `PASSAGE_AUTH_COOKIE_EXPIRY` |

How long authentication cookies remain valid in seconds. When a player connects with a valid cookie, Passage can skip the Mojang authentication step. See [Authentication Cookies](/advanced/cookies/) for details.

```yaml
auth_cookie_expiry: 21600  # 6 hours (default)
auth_cookie_expiry: 3600   # 1 hour (more frequent re-auth)
```

---

### `system_observer_interval`

| | |
|---|---|
| **Type** | integer (seconds, optional) |
| **Default** | `20` |
| **Environment** | `PASSAGE_SYSTEM_OBSERVER_INTERVAL` |

The interval in seconds at which system metrics (CPU, memory, swap) are observed and reported via OpenTelemetry. Set to `null` to disable system metric collection.

---

### `auth_secret`

| | |
|---|---|
| **Type** | string (optional) |
| **Default** | `null` (disabled) |
| **Environment** | `PASSAGE_AUTH_SECRET` |

Secret key for signing authentication cookies (HMAC-SHA256). If not set, authentication cookies are disabled and players must authenticate with Mojang on every connection.

**Recommended approach -- use a separate secret file:**
```bash
# Generate a secret
openssl rand -base64 32 > config/auth_secret

# Or specify custom path via environment variable
export AUTH_SECRET_FILE=/run/secrets/passage-auth
```

The `AUTH_SECRET_FILE` environment variable (default: `config/auth_secret`) points to a plain text file whose entire contents become the `auth_secret` value.

:::caution[Security]
- Use at least 32 characters
- Never commit secrets to version control
- In Kubernetes, use a Secret resource mounted as a file
- Rotate secrets periodically
:::

---

## Rate Limiter

| | |
|---|---|
| **Type** | object (optional) |
| **Enabled by** | Presence of the section |
| **Environment prefix** | `PASSAGE_RATE_LIMITER_` |

Per-IP connection rate limiting. If omitted, rate limiting is disabled.

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `duration` | integer (seconds) | `60` | Time window for counting connections. |
| `limit` | integer | `60` | Maximum connections allowed per IP within the time window. |

```yaml
rate_limiter:
  duration: 60   # 60-second window
  limit: 60      # max 60 connections per IP per minute
```

**Behavior:** If an IP exceeds `limit` connections within `duration` seconds, subsequent connections are rejected until the window expires.

**Tuning guidelines:**

| Scenario | `duration` | `limit` |
|----------|-----------|---------|
| Strict (DDoS protection) | `60` | `30` |
| Balanced (default) | `60` | `60` |
| Permissive (shared IPs) | `120` | `200` |

---

## PROXY Protocol

| | |
|---|---|
| **Type** | object (optional) |
| **Enabled by** | Presence of the section |
| **Environment prefix** | `PASSAGE_PROXY_PROTOCOL_` |

[PROXY protocol](https://www.haproxy.org/download/1.8/doc/proxy-protocol.txt) support for preserving real client IP addresses when Passage is behind a load balancer. If omitted, PROXY protocol is disabled.

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `allow_v1` | boolean | `true` | Accept PROXY protocol v1 (text) headers. |
| `allow_v2` | boolean | `true` | Accept PROXY protocol v2 (binary) headers. |

```yaml
proxy_protocol:
  allow_v1: true
  allow_v2: true
```

:::caution[Important]
Only enable PROXY protocol if **all** traffic to Passage includes PROXY protocol headers. Mixed traffic (some with headers, some without) will cause connection failures.
:::

**When to enable:**
- Behind HAProxy with `send-proxy` or `send-proxy-v2`
- Behind AWS Network Load Balancer (NLB) with proxy protocol enabled
- Behind NGINX with `proxy_protocol` configured
- Behind other PROXY protocol-compatible load balancers

---

## Sentry

| | |
|---|---|
| **Type** | object (optional) |
| **Enabled by** | Presence of the section |
| **Environment prefix** | `PASSAGE_SENTRY_` |

Error tracking with [Sentry](https://sentry.io). The release version is automatically inferred from the build. If omitted, Sentry is disabled.

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `debug` | boolean | `false` | Enable Sentry SDK debug logging. |
| `environment` | string | `""` | Environment tag for Sentry events (e.g., `"production"`, `"staging"`). |
| `address` | string | `""` | Sentry DSN (Data Source Name) URL. |

```yaml
sentry:
  debug: false
  environment: "production"
  address: "https://examplePublicKey@o0.ingest.sentry.io/0"
```

---

## OpenTelemetry

| | |
|---|---|
| **Type** | object |
| **Environment prefix** | `PASSAGE_OTEL_` |

OpenTelemetry configuration for traces, metrics, and logs. Each signal type (traces, metrics, logs) has its own endpoint and can be enabled independently.

### Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `environment` | string | `""` | Environment label added to all telemetry data. |
| `traces` | object (optional) | `null` | Traces endpoint configuration. |
| `metrics` | object (optional) | `null` | Metrics endpoint configuration. |
| `logs` | object (optional) | `null` | Logs endpoint configuration. |

Each endpoint object has:

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `address` | string | `""` | OTLP HTTP/protobuf endpoint URL. |
| `token` | string | `""` | Base64-encoded basic auth token. |

```yaml
otel:
  environment: "production"
  traces:
    address: "https://otlp-gateway.grafana.net/otlp/v1/traces"
    token: "base64_encoded_token"
  metrics:
    address: "https://otlp-gateway.grafana.net/otlp/v1/metrics"
    token: "base64_encoded_token"
  logs:
    address: "https://otlp-gateway.grafana.net/otlp/v1/logs"
    token: "base64_encoded_token"
```

**Generating a token:**
```bash
echo -n "user:password" | base64
```

**Supported backends:** Grafana Cloud, Datadog, New Relic, Honeycomb, or any OTLP-compatible collector.

See [Observability](/advanced/observability/) for detailed setup guides.

---

## Routes

| | |
|---|---|
| **Type** | array of route objects |
| **Default** | `[]` (empty) |

Routes define per-hostname adapter configurations. When a player connects, Passage matches the connection's hostname against each route's `hostname` regex pattern and uses the first match.

### Route Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `hostname` | string (regex) | `""` | Regex pattern to match the server hostname from the client handshake. |
| `status` | [StatusAdapter](#status-adapters) | `fixed` | Server list status configuration. |
| `authentication` | [AuthenticationAdapter](#authentication-adapters) | `mojang` | Player authentication configuration. |
| `discovery` | [DiscoveryAdapter](#discovery-adapter) | `fixed_discovery` | Backend server discovery and action pipeline. |
| `localization` | [LocalizationAdapter](#localization-adapters) | `fixed` | Disconnect message localization. |

```yaml
routes:
- hostname: "mc\\.example\\.net"
  status:
    type: fixed
    name: "My Network"
  authentication:
    type: mojang
  discovery:
    type: fixed_discovery
    targets:
    - identifier: "lobby-1"
      address: "10.0.1.10:25565"
  localization:
    type: fixed
    default_locale: "en"
```

:::tip[Hostname Matching]
The `hostname` field is a regex pattern. Use `\\.` to match literal dots. Use `.*` for a catch-all route. Routes are evaluated in order; the first match wins.
:::

---

## Status Adapters

Selected via `type` within `routes[].status`. See [Status Adapter](/adapters/status/) for detailed documentation.

### Fixed Status (`type: fixed`)

Static server status from configuration.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `name` | string | `"Passage"` | Server name in the server list. |
| `description` | string (optional) | `"\"Minecraft Server Transfer Router\""` | MOTD as JSON text component. |
| `favicon` | string (optional) | Passage logo | Base64-encoded PNG (`data:image/png;base64,...`). |
| `enforces_secure_chat` | boolean (optional) | `true` | Whether secure chat is enforced. |
| `preferred_version` | integer | `769` (1.21.4) | Protocol version shown to clients. |
| `min_version` | integer | `0` | Minimum supported protocol version. 0 = no minimum. |
| `max_version` | integer | `1000` | Maximum supported protocol version. |

```yaml
status:
  type: fixed
  name: "My Network"
  description: "{\"text\":\"Welcome!\",\"color\":\"gold\"}"
  enforces_secure_chat: true
  preferred_version: 769
  min_version: 766
  max_version: 1000
```

### HTTP Status (`type: http`)

Fetches status from an HTTP endpoint with caching.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `address` | string | `"http://localhost:8080"` | HTTP endpoint URL. |
| `cache_duration` | integer (seconds) | `60` | Cache duration. Must be greater than zero. |

```yaml
status:
  type: http
  address: "https://api.example.com/minecraft/status"
  cache_duration: 30
```

### gRPC Status (`type: grpc`)

Fetches status via a custom gRPC service.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `address` | string | `""` | gRPC service endpoint URL. |

```yaml
status:
  type: grpc
  address: "http://status-service:50051"
```

---

## Authentication Adapters

Selected via `type` within `routes[].authentication`. See [Authentication Adapter](/adapters/authentication/) for detailed documentation.

| Type | Description |
|------|-------------|
| `mojang` | Standard Mojang/Microsoft authentication (default). |
| `disabled` | No authentication. For testing only. |
| `fixed` | Fixed player profile for all connections. |
| `grpc` | Custom authentication via gRPC service. |

---

## Discovery Adapter

The discovery section has two parts: a **discovery adapter** (provides the initial target list) and an **actions pipeline** (transforms the list). The adapter type is set via `type`, and actions are listed in `actions[]`.

See [Target Discovery](/adapters/target-discovery/) and [Discovery Actions](/adapters/discovery-actions/) for detailed documentation.

### Fixed Discovery (`type: fixed_discovery`)

Static target list from configuration.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `targets` | array | `[]` | List of backend server targets. |

Each target:

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `identifier` | string | *required* | Unique server identifier. |
| `address` | string | *required* | Socket address (`host:port`). |
| `priority` | integer | `0` | Priority (lower = preferred). |
| `meta` | map | `{}` | Key-value metadata. |

```yaml
discovery:
  type: fixed_discovery
  targets:
  - identifier: "lobby-1"
    address: "10.0.1.10:25565"
    meta:
      type: "lobby"
      players: "15"
```

### DNS Discovery (`type: dns_discovery`)

Discovers targets via DNS SRV or A/AAAA records with periodic refresh.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `domain` | string | `""` | DNS domain to query. |
| `refresh_interval` | integer (seconds) | `30` | How often to re-query DNS. |
| `record_type` | string | `"srv"` | Record type: `"srv"` or `"a"`. |
| `port` | integer | `25565` | Default port (only for `record_type: a`). |

```yaml
# SRV records
discovery:
  type: dns_discovery
  domain: "_minecraft._tcp.servers.example.net"
  record_type: srv
  refresh_interval: 30

# A/AAAA records
discovery:
  type: dns_discovery
  domain: "mc.example.net"
  record_type: a
  port: 25565
  refresh_interval: 30
```

### Agones Discovery (`type: agones_discovery`)

Discovers game servers via [Agones](https://agones.dev/) GameServerAllocation in Kubernetes.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `namespace` | string (optional) | `null` | Kubernetes namespace. `null` = search all namespaces. |
| `selectors` | array | `[]` | GameServerAllocation selector templates. |
| `priorities` | array | `[]` | Priority templates for allocation. |
| `scheduling` | string (optional) | `null` | Agones scheduling strategy. |
| `metadata` | object (optional) | `null` | Metadata template for allocation. |
| `backoff` | object | see below | Exponential backoff configuration. |

**Template variables** (replaced in string fields if they exactly match):
- `{{ .Client.ProtocolVersion }}` -- client protocol version
- `{{ .Client.ServerAddress }}` -- server address from handshake
- `{{ .Client.ServerPort }}` -- server port from handshake
- `{{ .Client.Address }}` -- client IP (with optional proxy protocol)
- `{{ .Request.TraceId }}` -- OpenTelemetry trace ID

**Backoff configuration:**

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `initial_secs` | integer | `2` | Wait time for first retry. |
| `max_secs` | integer | `60` | Maximum wait time between retries. |
| `max_attempts` | integer | `10` | Maximum retry attempts. |
| `factor` | float | `2.0` | Multiplicative backoff factor. |
| `jitter` | float | `0.1` | Random jitter added (seconds). |

### gRPC Discovery (`type: grpc_discovery`)

Dynamic discovery via a custom gRPC service.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `address` | string | `""` | gRPC service endpoint URL. |

```yaml
discovery:
  type: grpc_discovery
  address: "http://discovery-service:50051"
```

### Discovery Actions (`actions`)

An optional array of actions that process the discovered target list sequentially. See [Discovery Actions](/adapters/discovery-actions/) for the full reference.

Available action types: `meta_filter`, `player_allow_filter`, `player_block_filter`, `player_fill_strategy`, `grpc`.

```yaml
discovery:
  type: dns_discovery
  domain: "servers.example.net"
  record_type: srv
  actions:
  - type: meta_filter
    rules:
    - key: "status"
      op: equals
      value: "online"
  - type: player_fill_strategy
    field: "players"
    max_players: 50
```

---

## Localization Adapters

Selected via `type` within `routes[].localization`. See [Localization](/advanced/localization/) for detailed documentation.

### Fixed Localization (`type: fixed`)

Static disconnect messages from configuration.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `default_locale` | string | `"en_US"` | Default locale for unknown clients. |
| `messages` | map | 6 built-in locales | Locale-specific messages. |
| `warn_unknown_keys` | boolean | `true` | Warn about unrecognized message keys in logs. |

**Default message keys:**

| Key | When shown |
|-----|-----------|
| `disconnect_timeout` | Connection timed out (keep-alive timeout). |
| `disconnect_no_target` | No backend server available. |
| `disconnect_unauthenticated` | Authentication failed. |

Messages use Minecraft JSON text component format:

```yaml
localization:
  type: fixed
  default_locale: "en"
  messages:
    en:
      locale: "English"
      disconnect_timeout: '{"text":"Disconnected: Connection timed out"}'
      disconnect_no_target: '{"text":"Disconnected: No server available"}'
      disconnect_unauthenticated: '{"text":"Disconnected: Authentication failed"}'
    de:
      locale: "Deutsch"
      disconnect_timeout: '{"text":"Verbindung getrennt: Zeitüberschreitung"}'
      disconnect_no_target: '{"text":"Verbindung getrennt: Kein Server verfügbar"}'
      disconnect_unauthenticated: '{"text":"Verbindung getrennt: Authentifizierung fehlgeschlagen"}'
```

### gRPC Localization (`type: grpc`)

Delegates localization to a custom gRPC service.

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `address` | string | `""` | gRPC service endpoint URL. |

---

## Complete Example

A production-ready configuration with DNS discovery, rate limiting, and observability:

```yaml
# yaml-language-server: $schema=./schema.json

address: "0.0.0.0:25565"
timeout: 120

rate_limiter:
  duration: 60
  limit: 60

proxy_protocol:
  allow_v1: true
  allow_v2: true

sentry:
  environment: "production"
  address: "https://examplePublicKey@o0.ingest.sentry.io/0"

otel:
  environment: "production"
  traces:
    address: "https://otlp-gateway.grafana.net/otlp/v1/traces"
    token: "base64_token"
  metrics:
    address: "https://otlp-gateway.grafana.net/otlp/v1/metrics"
    token: "base64_token"

routes:
- hostname: "mc\\.example\\.net"
  status:
    type: http
    address: "https://example.net/status"
    cache_duration: 30
  authentication:
    type: mojang
    server_id: ""
  discovery:
    type: dns_discovery
    domain: "servers.example.net"
    record_type: srv
    actions:
    - type: meta_filter
      name: "server-filter"
      rules:
      - key: "status"
        op: equals
        value: "online"
    - type: player_fill_strategy
      name: "player-fill"
      field: "players"
      max_players: 50
  localization:
    type: fixed
    default_locale: "en"
```

---

## Configuration Layers

Passage uses a layered configuration system. Upper layers override lower layers:

1. **Environment variables** (highest priority) -- format: `PASSAGE_<FIELD>` with `_` as separator
2. **Auth secret file** -- sets only `auth_secret` (default path: `config/auth_secret`)
3. **Configuration file** -- your deployment config (default path: `config/config`)
4. **Default values** (lowest priority) -- built into Passage

### Environment Variables

Override any configuration value:

```bash
export PASSAGE_ADDRESS="0.0.0.0:25565"
export PASSAGE_TIMEOUT=120
export PASSAGE_RATE_LIMITER_DURATION=60
export PASSAGE_RATE_LIMITER_LIMIT=100
```

Change the environment variable prefix:
```bash
export ENV_PREFIX=MYAPP
export MYAPP_ADDRESS="0.0.0.0:25565"
```

### Configuration File Formats

Passage auto-detects the format from the file extension:

```bash
CONFIG_FILE=config/config.yaml passage    # YAML
CONFIG_FILE=config/config.toml passage    # TOML
CONFIG_FILE=config/config.json passage    # JSON
```
