---
title: Configuration
description: Complete guide to configuring Passage for your Minecraft network.
---

This page provides a complete reference for all configuration options in Passage. For a beginner-friendly introduction, see [Configuration Basics](/setup/configuration-basics/).

## Configuration Structure

Passage uses TOML format with the following top-level sections:

```toml
# Core connection settings
address = "0.0.0.0:25565"
timeout = 120

# Security and monitoring
[sentry]
[otel]
[rate_limiter]
[proxy_protocol]

# Adapter configuration
[status]
[target_discovery]
[target_strategy]

# Internationalization
[localization]
```

## Core Settings

### `address`

**Type:** String (socket address)
**Required:** Yes
**Default:** `"0.0.0.0:25565"`

The network address Passage binds to for incoming Minecraft connections.

```toml
# Listen on all interfaces, port 25565
address = "0.0.0.0:25565"

# Listen on specific interface
address = "192.168.1.100:25565"

# Use different port
address = "0.0.0.0:25566"
```

### `timeout`

**Type:** Number (seconds)
**Required:** Yes
**Default:** `120`

Maximum time in seconds to wait for client responses during connection handshake.

```toml
# Standard timeout
timeout = 120

# Shorter timeout for high-performance scenarios
timeout = 60

# Longer timeout for slow connections
timeout = 300
```

---

## Security Settings

### `[rate_limiter]`

Prevents connection flooding by limiting connections per IP address.

#### `enabled`

**Type:** Boolean
**Default:** `true`

Enable or disable rate limiting.

#### `duration`

**Type:** Number (seconds)
**Default:** `60`

Time window for connection counting.

#### `size`

**Type:** Number
**Default:** `60`

Maximum connections allowed per IP within the duration window.

```toml
[rate_limiter]
enabled = true
duration = 60  # 60 second window
size = 60      # Max 60 connections per IP per minute
```

**Example:** With `duration=60` and `size=60`, an IP can make 60 connections per minute. If they try a 61st connection within that minute, it will be rejected.

### `[proxy_protocol]`

Enables PROXY protocol support for getting real client IPs behind load balancers.

#### `enabled`

**Type:** Boolean
**Default:** `false`

Enable PROXY protocol v1/v2 support.

```toml
[proxy_protocol]
enabled = true
```

Use when Passage is behind:
- HAProxy
- AWS Network Load Balancer (NLB)
- NGINX (with proxy_protocol)
- Other PROXY protocol-compatible load balancers

### `auth_secret`

**Type:** String
**Optional**
**Default:** None

Secret key for authentication cookie signing. If not set, cookies are disabled.

```toml
# In config.toml
auth_secret = "your-secret-key-here"
```

**Better approach:** Use a separate file:

```bash
# Create secret file
echo "your-secret-key-here" > config/auth_secret

# No config needed - automatically loaded
```

Or via environment variable:

```bash
AUTH_SECRET_FILE=/run/secrets/passage-auth passage
```

---

## Observability Settings

### `[sentry]`

Configure Sentry error tracking (optional feature).

#### `enabled`

**Type:** Boolean
**Default:** `false`

#### `debug`

**Type:** Boolean
**Default:** `false`

#### `address`

**Type:** String (URL)
**Required if enabled**

Sentry DSN URL.

#### `environment`

**Type:** String
**Default:** `"staging"`

Environment name for Sentry events.

```toml
[sentry]
enabled = true
debug = false
address = "https://your-key@sentry.io/project-id"
environment = "production"
```

### `[otel]`

Configure OpenTelemetry for metrics and tracing.

#### `environment`

**Type:** String
**Default:** `"production"`

Environment label for telemetry data.

#### `traces_endpoint`

**Type:** String (URL)
**Required**

OTLP HTTP endpoint for traces.

#### `traces_token`

**Type:** String
**Required**

Base64-encoded basic auth token for traces endpoint.

#### `metrics_endpoint`

**Type:** String (URL)
**Required**

OTLP HTTP endpoint for metrics.

#### `metrics_token`

**Type:** String
**Required**

Base64-encoded basic auth token for metrics endpoint.

```toml
[otel]
environment = "production"
traces_endpoint = "https://otlp-gateway.grafana.net/otlp/v1/traces"
traces_token = "base64-token-here"
metrics_endpoint = "https://otlp-gateway.grafana.net/otlp/v1/metrics"
metrics_token = "base64-token-here"
```

For Grafana Cloud, get tokens from: **Configuration → Data Sources → OpenTelemetry**.

---

## Adapter Configuration

### `[status]`

Configure how Passage responds to server list pings.

#### `adapter`

**Type:** String
**Required:** Yes
**Values:** `"fixed"`, `"http"`, `"grpc"`

See [Status Adapters](/customization/status-adapters/) for detailed documentation.

#### Fixed Adapter

```toml
[status]
adapter = "fixed"

[status.fixed]
name = "My Network"
description = "\"Welcome!\""
favicon = "data:image/png;base64,..."
enforces_secure_chat = true
preferred_version = 769
min_version = 766
max_version = 1000
```

#### HTTP Adapter

```toml
[status]
adapter = "http"

[status.http]
address = "https://api.example.com/status"
cache_duration = 5
```

#### gRPC Adapter

```toml
[status]
adapter = "grpc"

[status.grpc]
address = "http://status-service:3030"
```

### `[target_discovery]`

Configure how Passage discovers available backend servers.

#### `adapter`

**Type:** String
**Required:** Yes
**Values:** `"fixed"`, `"grpc"`, `"agones"`

See [Target Discovery Adapters](/customization/target-discovery-adapters/) for detailed documentation.

#### Fixed Adapter

```toml
[target_discovery]
adapter = "fixed"

[[target_discovery.fixed.targets]]
identifier = "hub-1"
address = "10.0.1.10:25565"
meta = { type = "hub", players = "15" }

[[target_discovery.fixed.targets]]
identifier = "survival-1"
address = "10.0.2.10:25565"
meta = { type = "survival" }
```

#### gRPC Adapter

```toml
[target_discovery]
adapter = "grpc"

[target_discovery.grpc]
address = "http://discovery-service:3030"
```

#### Agones Adapter

```toml
[target_discovery]
adapter = "agones"

[target_discovery.agones]
namespace = "minecraft"
label_selector = "game=minecraft,type=lobby"
```

### `[target_strategy]`

Configure how Passage selects which server to send each player to.

#### `adapter`

**Type:** String
**Required:** Yes
**Values:** `"fixed"`, `"player_fill"`, `"grpc"`

See [Target Strategy Adapters](/customization/target-strategy-adapters/) for detailed documentation.

#### Fixed Adapter

```toml
[target_strategy]
adapter = "fixed"
```

#### Player Fill Adapter

```toml
[target_strategy]
adapter = "player_fill"

[target_strategy.player_fill]
field = "players"
max_players = 50

[[target_strategy.player_fill.target_filters]]
server_host = "lobby.example.com"
meta = { type = "lobby" }
```

#### gRPC Adapter

```toml
[target_strategy]
adapter = "grpc"

[target_strategy.grpc]
address = "http://strategy-service:3030"
```

---

## Localization

### `[localization]`

Configure disconnect messages in multiple languages.

#### `default_locale`

**Type:** String
**Default:** `"en_US"`

Default locale when client locale is unknown.

#### `messages`

**Type:** Nested tables
**Structure:** `[localization.messages.{locale}]`

Each locale is a table of message keys to JSON text values.

```toml
[localization]
default_locale = "en_US"

[localization.messages.en]
disconnect_timeout = "{\"text\":\"Connection timeout\"}"
disconnect_no_target = "{\"text\":\"No server available\"}"

[localization.messages.es]
disconnect_timeout = "{\"text\":\"Tiempo de espera agotado\"}"
disconnect_no_target = "{\"text\":\"Servidor no disponible\"}"

[localization.messages.de]
disconnect_timeout = "{\"text\":\"Verbindungszeitüberschreitung\"}"
disconnect_no_target = "{\"text\":\"Kein Server verfügbar\"}"
```

#### Message Keys

- `disconnect_timeout`: Shown when connection times out
- `disconnect_no_target`: Shown when no backend server is available
- `disconnect_failed_resourcepack`: Shown when resource pack fails to load

Messages support parameter substitution:
```toml
disconnect_custom = "{\"text\":\"Hello {player}!\"}"
```

---

## Complete Example Configurations

### Minimal Setup

```toml
address = "0.0.0.0:25565"
timeout = 120

[status]
adapter = "fixed"
[status.fixed]
name = "My Server"

[target_discovery]
adapter = "fixed"
[[target_discovery.fixed.targets]]
identifier = "main"
address = "127.0.0.1:25566"

[target_strategy]
adapter = "fixed"

[localization]
default_locale = "en_US"
```

### Production Setup

```toml
address = "0.0.0.0:25565"
timeout = 120

[rate_limiter]
enabled = true
duration = 60
size = 100

[proxy_protocol]
enabled = true

[sentry]
enabled = true
debug = false
address = "https://key@sentry.io/project"
environment = "production"

[otel]
environment = "production"
traces_endpoint = "https://traces.grafana.net/otlp/v1/traces"
traces_token = "token"
metrics_endpoint = "https://metrics.grafana.net/otlp/v1/metrics"
metrics_token = "token"

[status]
adapter = "http"
[status.http]
address = "https://status.example.com/minecraft"
cache_duration = 5

[target_discovery]
adapter = "agones"
[target_discovery.agones]
namespace = "minecraft"
label_selector = "game=minecraft"

[target_strategy]
adapter = "player_fill"
[target_strategy.player_fill]
field = "players"
max_players = 50

[localization]
default_locale = "en_US"
```

## Environment Variable Override

All config values can be overridden with environment variables:

```bash
# Format: PASSAGE_{SECTION}_{FIELD}
export PASSAGE_ADDRESS="0.0.0.0:25565"
export PASSAGE_TIMEOUT=120
export PASSAGE_STATUS_ADAPTER="fixed"
export PASSAGE_STATUS_FIXED_NAME="My Server"
export PASSAGE_RATE_LIMITER_ENABLED=true
export PASSAGE_RATE_LIMITER_SIZE=100
```
