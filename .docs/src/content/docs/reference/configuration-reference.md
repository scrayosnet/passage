---
title: Configuration Reference
description: Complete field-by-field reference for all Passage configuration options.
---

This page provides a comprehensive reference for every configuration field in Passage. For practical examples and guides, see [Configuration Basics](/setup/configuration-basics/).

## Table of Contents

- [Core Settings](#core-settings)
- [Security Settings](#security-settings)
- [Observability Settings](#observability-settings)
- [Adapter Configuration](#adapter-configuration)
- [Localization](#localization)

## Core Settings

### `address`

**Type:** `string` (socket address)
**Required:** Yes
**Default:** `"0.0.0.0:25565"`
**Environment:** `PASSAGE_ADDRESS`

The network address and port that Passage binds to for incoming Minecraft client connections.

**Format:** `"<host>:<port>"`

**Examples:**
```toml
# Listen on all interfaces, standard Minecraft port
address = "0.0.0.0:25565"

# Listen on specific interface
address = "192.168.1.100:25565"

# Use custom port
address = "0.0.0.0:19132"
```

**Notes:**
- Use `0.0.0.0` to listen on all network interfaces
- Use `127.0.0.1` to listen only on localhost
- Ensure the port is not already in use
- Requires appropriate firewall configuration

---

### `timeout`

**Type:** `integer` (seconds)
**Required:** Yes
**Default:** `120`
**Environment:** `PASSAGE_TIMEOUT`

Maximum time in seconds to wait for client responses during the connection handshake process.

**Range:** `1` to `3600`

**Examples:**
```toml
# Standard timeout (2 minutes)
timeout = 120

# Short timeout for high-performance scenarios
timeout = 60

# Long timeout for slow connections
timeout = 300
```

**Recommendations:**
- **Production:** 120-180 seconds
- **High-performance:** 60 seconds
- **Mobile/slow connections:** 240-300 seconds

---

### `auth_secret`

**Type:** `string`
**Required:** No
**Default:** None
**Environment:** `PASSAGE_AUTH_SECRET`

Secret key for signing authentication cookies. If not set, authentication cookies are disabled.

**Examples:**
```toml
# In config.toml (not recommended for production)
auth_secret = "your-secret-key-here"
```

**Better approach - using a separate file:**
```bash
# Create secret file
echo "your-secret-key-here" > config/auth_secret

# Or specify custom path
export AUTH_SECRET_FILE=/run/secrets/passage-auth
```

**Security Best Practices:**
- Use at least 32 characters
- Generate with: `openssl rand -base64 32`
- Never commit secrets to version control
- Use file-based secrets in production
- Rotate secrets periodically

---

## Security Settings

### `[rate_limiter]`

Connection rate limiting to prevent abuse and DoS attacks.

#### `rate_limiter.enabled`

**Type:** `boolean`
**Default:** `true`
**Environment:** `PASSAGE_RATE_LIMITER_ENABLED`

Enable or disable rate limiting entirely.

```toml
[rate_limiter]
enabled = true
```

#### `rate_limiter.duration`

**Type:** `integer` (seconds)
**Default:** `60`
**Environment:** `PASSAGE_RATE_LIMITER_DURATION`

Time window for counting connections from each IP address.

**Range:** `1` to `3600`

#### `rate_limiter.size`

**Type:** `integer`
**Default:** `60`
**Environment:** `PASSAGE_RATE_LIMITER_SIZE`

Maximum number of connections allowed per IP address within the duration window.

**Range:** `1` to `1000`

**Example:**
```toml
[rate_limiter]
enabled = true
duration = 60   # 60 second window
size = 60       # Max 60 connections per IP per minute
```

**Behavior:** If an IP makes more than `size` connections within `duration` seconds, subsequent connections are rejected until the window expires.

**Tuning:**
- **Strict:** `duration=60`, `size=30`
- **Balanced:** `duration=60`, `size=60` (default)
- **Permissive:** `duration=120`, `size=200`

---

### `[proxy_protocol]`

PROXY protocol support for preserving real client IP addresses when behind load balancers.

#### `proxy_protocol.enabled`

**Type:** `boolean`
**Default:** `false`
**Environment:** `PASSAGE_PROXY_PROTOCOL_ENABLED`

Enable PROXY protocol v1 and v2 support.

```toml
[proxy_protocol]
enabled = true
```

**When to enable:**
- Behind HAProxy
- Behind AWS Network Load Balancer (NLB)
- Behind NGINX with `proxy_protocol` enabled
- Behind other PROXY protocol-compatible load balancers

**Important:** Only enable if ALL traffic will include PROXY protocol headers. Mixed traffic will cause connection failures.

---

## Observability Settings

### `[sentry]`

Error tracking integration with Sentry.io.

#### `sentry.enabled`

**Type:** `boolean`
**Default:** `false`
**Environment:** `PASSAGE_SENTRY_ENABLED`

Enable Sentry error tracking.

#### `sentry.debug`

**Type:** `boolean`
**Default:** `false`
**Environment:** `PASSAGE_SENTRY_DEBUG`

Enable debug logging for Sentry SDK.

#### `sentry.address`

**Type:** `string` (URL)
**Required:** Yes (if enabled)
**Environment:** `PASSAGE_SENTRY_ADDRESS`

Sentry DSN (Data Source Name) URL.

**Format:** `https://<key>@<organization>.ingest.sentry.io/<project-id>`

#### `sentry.environment`

**Type:** `string`
**Default:** `"staging"`
**Environment:** `PASSAGE_SENTRY_ENVIRONMENT`

Environment tag for Sentry events.

**Example:**
```toml
[sentry]
enabled = true
debug = false
address = "https://examplePublicKey@o0.ingest.sentry.io/0"
environment = "production"
```

---

### `[otel]`

OpenTelemetry configuration for metrics and distributed tracing.

#### `otel.environment`

**Type:** `string`
**Default:** `"production"`
**Environment:** `PASSAGE_OTEL_ENVIRONMENT`

Environment label added to all telemetry data.

#### `otel.traces_endpoint`

**Type:** `string` (URL)
**Required:** Yes (for tracing)
**Environment:** `PASSAGE_OTEL_TRACES_ENDPOINT`

OTLP HTTP endpoint for trace data.

**Format:** `https://<host>/v1/traces`

#### `otel.traces_token`

**Type:** `string`
**Required:** Yes (for tracing)
**Environment:** `PASSAGE_OTEL_TRACES_TOKEN`

Base64-encoded basic auth token for traces endpoint.

**Generation:**
```bash
echo -n "user:password" | base64
```

#### `otel.metrics_endpoint`

**Type:** `string` (URL)
**Required:** Yes (for metrics)
**Environment:** `PASSAGE_OTEL_METRICS_ENDPOINT`

OTLP HTTP endpoint for metrics data.

**Format:** `https://<host>/v1/metrics`

#### `otel.metrics_token`

**Type:** `string`
**Required:** Yes (for metrics)
**Environment:** `PASSAGE_OTEL_METRICS_TOKEN`

Base64-encoded basic auth token for metrics endpoint.

**Example (Grafana Cloud):**
```toml
[otel]
environment = "production"
traces_endpoint = "https://otlp-gateway-prod-eu-west-0.grafana.net/otlp/v1/traces"
traces_token = "base64_encoded_token_here"
metrics_endpoint = "https://otlp-gateway-prod-eu-west-0.grafana.net/otlp/v1/metrics"
metrics_token = "base64_encoded_token_here"
```

**Supported Backends:**
- Grafana Cloud
- Datadog
- New Relic
- Honeycomb
- Any OTLP-compatible backend

---

## Adapter Configuration

### `[status]`

Server status response configuration (for server list pings).

#### `status.adapter`

**Type:** `string`
**Required:** Yes
**Environment:** `PASSAGE_STATUS_ADAPTER`
**Values:** `"fixed"`, `"http"`, `"grpc"`

The adapter type for status responses.

For detailed information, see [Status Adapters](/customization/status-adapters/).

---

#### Fixed Status Adapter

Static server status configuration.

##### `status.fixed.name`

**Type:** `string`
**Required:** Yes
**Environment:** `PASSAGE_STATUS_FIXED_NAME`

Server name shown in the server list.

##### `status.fixed.description`

**Type:** `string`
**Default:** `"\"A Minecraft server powered by Passage\""`
**Environment:** `PASSAGE_STATUS_FIXED_DESCRIPTION`

MOTD (Message of the Day) as JSON text component.

**Format:** Minecraft JSON text format (escaped)

##### `status.fixed.favicon`

**Type:** `string`
**Default:** None
**Environment:** `PASSAGE_STATUS_FIXED_FAVICON`

Server icon as base64-encoded PNG.

**Format:** `data:image/png;base64,<base64_data>`

##### `status.fixed.enforces_secure_chat`

**Type:** `boolean`
**Default:** `true`
**Environment:** `PASSAGE_STATUS_FIXED_ENFORCES_SECURE_CHAT`

Whether server enforces secure chat (1.19+).

##### `status.fixed.preferred_version`

**Type:** `integer`
**Default:** `769` (1.21.4)
**Environment:** `PASSAGE_STATUS_FIXED_PREFERRED_VERSION`

Preferred protocol version shown to clients.

##### `status.fixed.min_version`

**Type:** `integer`
**Default:** `0`
**Environment:** `PASSAGE_STATUS_FIXED_MIN_VERSION`

Minimum supported protocol version. 0 means no minimum.

##### `status.fixed.max_version`

**Type:** `integer`
**Default:** `0`
**Environment:** `PASSAGE_STATUS_FIXED_MAX_VERSION`

Maximum supported protocol version. 0 means no maximum.

**Example:**
```toml
[status]
adapter = "fixed"

[status.fixed]
name = "My Minecraft Network"
description = "{\"text\":\"Welcome!\",\"color\":\"gold\"}"
favicon = "data:image/png;base64,iVBORw0KGg..."
enforces_secure_chat = true
preferred_version = 769  # 1.21.4
min_version = 766        # 1.20.5
max_version = 1000       # Future versions
```

---

#### HTTP Status Adapter

Fetch status from HTTP endpoint.

##### `status.http.address`

**Type:** `string` (URL)
**Required:** Yes
**Environment:** `PASSAGE_STATUS_HTTP_ADDRESS`

HTTP endpoint URL.

##### `status.http.cache_duration`

**Type:** `integer` (seconds)
**Default:** `5`
**Environment:** `PASSAGE_STATUS_HTTP_CACHE_DURATION`

How long to cache responses.

**Example:**
```toml
[status]
adapter = "http"

[status.http]
address = "https://api.example.com/minecraft/status"
cache_duration = 5
```

---

#### gRPC Status Adapter

Fetch status via gRPC.

##### `status.grpc.address`

**Type:** `string` (URL)
**Required:** Yes
**Environment:** `PASSAGE_STATUS_GRPC_ADDRESS`

gRPC service address.

**Format:** `http://<host>:<port>` or `https://<host>:<port>`

**Example:**
```toml
[status]
adapter = "grpc"

[status.grpc]
address = "http://status-service:3030"
```

---

### `[target_discovery]`

Backend server discovery configuration.

#### `target_discovery.adapter`

**Type:** `string`
**Required:** Yes
**Environment:** `PASSAGE_TARGET_DISCOVERY_ADAPTER`
**Values:** `"fixed"`, `"grpc"`, `"agones"`

The adapter type for discovering backend servers.

For detailed information, see [Target Discovery Adapters](/customization/target-discovery-adapters/).

---

#### Fixed Discovery Adapter

Static list of backend servers.

##### `target_discovery.fixed.targets`

**Type:** `array of tables`
**Required:** Yes

List of backend server targets.

**Fields:**
- `identifier` (string, required): Unique server ID
- `address` (string, required): Socket address `"host:port"`
- `meta` (table, optional): Key-value metadata

**Example:**
```toml
[target_discovery]
adapter = "fixed"

[[target_discovery.fixed.targets]]
identifier = "hub-1"
address = "10.0.1.10:25565"
meta = { type = "hub", region = "us-east", players = "15" }

[[target_discovery.fixed.targets]]
identifier = "survival-1"
address = "10.0.2.10:25565"
meta = { type = "survival", region = "us-east" }
```

---

#### gRPC Discovery Adapter

Dynamic discovery via gRPC service.

##### `target_discovery.grpc.address`

**Type:** `string` (URL)
**Required:** Yes
**Environment:** `PASSAGE_TARGET_DISCOVERY_GRPC_ADDRESS`

gRPC discovery service address.

**Example:**
```toml
[target_discovery]
adapter = "grpc"

[target_discovery.grpc]
address = "http://discovery-service:3030"
```

---

#### Agones Discovery Adapter

Discover game servers via Agones Kubernetes integration.

##### `target_discovery.agones.namespace`

**Type:** `string`
**Default:** `"default"`
**Environment:** `PASSAGE_TARGET_DISCOVERY_AGONES_NAMESPACE`

Kubernetes namespace to search for GameServers.

##### `target_discovery.agones.label_selector`

**Type:** `string`
**Default:** `""`
**Environment:** `PASSAGE_TARGET_DISCOVERY_AGONES_LABEL_SELECTOR`

Kubernetes label selector to filter GameServers.

**Example:**
```toml
[target_discovery]
adapter = "agones"

[target_discovery.agones]
namespace = "minecraft"
label_selector = "game=minecraft,type=lobby"
```

---

### `[target_strategy]`

Server selection strategy configuration.

#### `target_strategy.adapter`

**Type:** `string`
**Required:** Yes
**Environment:** `PASSAGE_TARGET_STRATEGY_ADAPTER`
**Values:** `"fixed"`, `"player_fill"`, `"grpc"`

The adapter type for selecting which server to route players to.

For detailed information, see [Target Strategy Adapters](/customization/target-strategy-adapters/).

---

#### Fixed Strategy Adapter

Always select the first available server.

**Example:**
```toml
[target_strategy]
adapter = "fixed"
```

---

#### Player Fill Strategy Adapter

Route to server with lowest player count.

##### `target_strategy.player_fill.field`

**Type:** `string`
**Default:** `"players"`
**Environment:** `PASSAGE_TARGET_STRATEGY_PLAYER_FILL_FIELD`

Metadata field name containing player count.

##### `target_strategy.player_fill.max_players`

**Type:** `integer`
**Default:** `100`
**Environment:** `PASSAGE_TARGET_STRATEGY_PLAYER_FILL_MAX_PLAYERS`

Maximum players per server (servers at this limit are skipped).

##### `target_strategy.player_fill.target_filters`

**Type:** `array of tables`
**Default:** `[]`

Filters to match servers for specific criteria.

**Filter Fields:**
- `server_host` (string, optional): Match server hostname from handshake
- `meta` (table, optional): Match metadata key-value pairs

**Example:**
```toml
[target_strategy]
adapter = "player_fill"

[target_strategy.player_fill]
field = "players"
max_players = 50

# Route lobby.example.com to lobby servers
[[target_strategy.player_fill.target_filters]]
server_host = "lobby.example.com"
meta = { type = "lobby" }

# Route play.example.com to game servers
[[target_strategy.player_fill.target_filters]]
server_host = "play.example.com"
meta = { type = "game" }
```

---

#### gRPC Strategy Adapter

Custom selection logic via gRPC service.

##### `target_strategy.grpc.address`

**Type:** `string` (URL)
**Required:** Yes
**Environment:** `PASSAGE_TARGET_STRATEGY_GRPC_ADDRESS`

gRPC strategy service address.

**Example:**
```toml
[target_strategy]
adapter = "grpc"

[target_strategy.grpc]
address = "http://strategy-service:3030"
```

---

## Localization

### `[localization]`

Multi-language disconnect message configuration.

#### `localization.default_locale`

**Type:** `string`
**Default:** `"en_US"`
**Environment:** `PASSAGE_LOCALIZATION_DEFAULT_LOCALE`

Default locale code when client locale is unknown.

**Format:** `"<language>_<REGION>"` (e.g., `"en_US"`, `"es_ES"`, `"de_DE"`)

#### `localization.messages`

**Type:** `nested tables`
**Structure:** `[localization.messages.<locale>]`

Disconnect messages for each locale.

**Message Keys:**
- `disconnect_timeout`: Shown on connection timeout
- `disconnect_no_target`: Shown when no backend server available
- `disconnect_failed_resourcepack`: Shown on resource pack failure

**Value Format:** Minecraft JSON text component (escaped string)

**Placeholders:**
- `{player}`: Player username
- `{server}`: Server hostname
- `{reason}`: Disconnect reason

**Example:**
```toml
[localization]
default_locale = "en_US"

[localization.messages.en]
disconnect_timeout = "{\"text\":\"Connection timeout\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"No server available\",\"color\":\"yellow\"}"
disconnect_failed_resourcepack = "{\"text\":\"Failed to load resource pack\"}"

[localization.messages.es]
disconnect_timeout = "{\"text\":\"Tiempo de espera agotado\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"Servidor no disponible\",\"color\":\"yellow\"}"

[localization.messages.de]
disconnect_timeout = "{\"text\":\"Verbindungszeitüberschreitung\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"Kein Server verfügbar\",\"color\":\"yellow\"}"

[localization.messages.fr]
disconnect_timeout = "{\"text\":\"Délai de connexion dépassé\",\"color\":\"red\"}"
disconnect_no_target = "{\"text\":\"Aucun serveur disponible\",\"color\":\"yellow\"}"
```

---

## Environment Variable Mapping

All configuration values can be overridden with environment variables using the format:

```
PASSAGE_<SECTION>_<SUBSECTION>_<FIELD>=value
```

**Examples:**
```bash
# Core settings
export PASSAGE_ADDRESS="0.0.0.0:25565"
export PASSAGE_TIMEOUT=120

# Nested settings
export PASSAGE_RATE_LIMITER_ENABLED=true
export PASSAGE_RATE_LIMITER_SIZE=100

# Adapter settings
export PASSAGE_STATUS_ADAPTER="fixed"
export PASSAGE_STATUS_FIXED_NAME="My Server"
export PASSAGE_TARGET_DISCOVERY_ADAPTER="grpc"
export PASSAGE_TARGET_DISCOVERY_GRPC_ADDRESS="http://localhost:3030"

# Custom prefix
export ENV_PREFIX=MYAPP
export MYAPP_ADDRESS="0.0.0.0:25565"
```

---

## Configuration File Formats

Passage supports multiple configuration file formats:

### TOML (default)
```bash
CONFIG_FILE=config/config.toml passage
```

### JSON
```bash
CONFIG_FILE=config/config.json passage
```

### YAML
```bash
CONFIG_FILE=config/config.yaml passage
```

---

## Next Steps

- Learn about [Status Adapters](/customization/status-adapters/)
- Configure [Target Discovery](/customization/target-discovery-adapters/)
- Set up [Target Strategies](/customization/target-strategy-adapters/)
- See practical examples in [Configuration Basics](/setup/configuration-basics/)
