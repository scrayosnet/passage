---
title: Monitoring and Observability
description: Set up monitoring for Passage with OpenTelemetry and Sentry.
sidebar:
    order: 3
---

Passage provides built-in observability through:

- **OpenTelemetry** -- Traces, metrics, and logs via OTLP over HTTP
- **Sentry** (optional) -- Error tracking and crash reporting
- **Structured Logging** -- Configurable log levels via `RUST_LOG`

## OpenTelemetry

Passage natively exports traces, metrics, and logs using the OpenTelemetry Protocol (OTLP/HTTP). Each signal type (traces, metrics, logs) is independently configured with its own endpoint and authentication token.

### Configuration

```yaml
otel:
  environment: "production"
  traces:
    address: "https://otlp-gateway.example.com/v1/traces"
    token: "base64_auth_token"
  metrics:
    address: "https://otlp-gateway.example.com/v1/metrics"
    token: "base64_auth_token"
  logs:
    address: "https://otlp-gateway.example.com/v1/logs"
    token: "base64_auth_token"
```

Each endpoint (`traces`, `metrics`, `logs`) is optional. Only configure the signals you need.

| Field | Type | Description |
|-------|------|-------------|
| `environment` | string | Environment label attached to all telemetry |
| `traces.address` | string | OTLP/HTTP endpoint for traces |
| `traces.token` | string | Base64-encoded Basic Auth token for traces |
| `metrics.address` | string | OTLP/HTTP endpoint for metrics |
| `metrics.token` | string | Base64-encoded Basic Auth token for metrics |
| `logs.address` | string | OTLP/HTTP endpoint for logs |
| `logs.token` | string | Base64-encoded Basic Auth token for logs |

### Environment Variables

```bash
export PASSAGE_OTEL_ENVIRONMENT=production
export PASSAGE_OTEL_TRACES_ADDRESS=https://otlp.example.com/v1/traces
export PASSAGE_OTEL_TRACES_TOKEN=base64_token
export PASSAGE_OTEL_METRICS_ADDRESS=https://otlp.example.com/v1/metrics
export PASSAGE_OTEL_METRICS_TOKEN=base64_token
export PASSAGE_OTEL_LOGS_ADDRESS=https://otlp.example.com/v1/logs
export PASSAGE_OTEL_LOGS_TOKEN=base64_token
```

### Grafana Cloud Example

1. Navigate to **Configuration > Data Sources > OpenTelemetry** and copy your OTLP endpoint URLs
2. Generate an auth token:
   ```bash
   echo -n "instanceID:apiKey" | base64
   ```
3. Configure Passage:
   ```yaml
   otel:
     environment: "production"
     traces:
       address: "https://otlp-gateway-prod-us-central-0.grafana.net/otlp/v1/traces"
       token: "MTIzNDU6Z2xjX3h4eHh4"
     metrics:
       address: "https://otlp-gateway-prod-us-central-0.grafana.net/otlp/v1/metrics"
       token: "MTIzNDU6Z2xjX3h4eHh4"
     logs:
       address: "https://otlp-gateway-prod-us-central-0.grafana.net/otlp/v1/logs"
       token: "MTIzNDU6Z2xjX3h4eHh4"
   ```

### Self-Hosted with OpenTelemetry Collector

If you're running Prometheus, Jaeger, or Loki locally, use an OTel Collector as a gateway:

```yaml
# otel-collector-config.yaml
receivers:
  otlp:
    protocols:
      http:
        endpoint: "0.0.0.0:4318"

processors:
  batch:

exporters:
  prometheus:
    endpoint: "0.0.0.0:8889"
  otlp/jaeger:
    endpoint: "jaeger:4317"
    tls:
      insecure: true

service:
  pipelines:
    metrics:
      receivers: [otlp]
      processors: [batch]
      exporters: [prometheus]
    traces:
      receivers: [otlp]
      processors: [batch]
      exporters: [otlp/jaeger]
```

Then point Passage at the collector:

```yaml
otel:
  environment: "production"
  traces:
    address: "http://otel-collector:4318/v1/traces"
  metrics:
    address: "http://otel-collector:4318/v1/metrics"
```

---

## Sentry Error Tracking

Sentry captures panics, errors, and unexpected failures. It is **enabled by adding the `sentry` section** -- there is no separate `enabled` field.

```yaml
sentry:
  address: "https://examplePublicKey@o0.ingest.sentry.io/0"
  environment: "production"
  debug: false
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `address` | string | `""` | Sentry DSN (Data Source Name) |
| `environment` | string | `""` | Environment label for grouping events |
| `debug` | bool | `false` | Enable Sentry debug mode |

Sentry captures panics, adapter errors, connection failures, and configuration errors with full stack traces and request context.

---

## System Observer

Passage can periodically report system-level metrics (CPU, memory, etc.):

```yaml
# Interval in seconds (default: enabled, set to null to disable)
system_observer_interval: 15
```

---

## Logging

Passage uses structured logging configurable via the `RUST_LOG` environment variable:

```bash
# Error only
RUST_LOG=error passage

# Info (default)
RUST_LOG=info passage

# Debug (includes adapter communication details)
RUST_LOG=debug passage

# Trace (very verbose, includes packet-level details)
RUST_LOG=trace passage

# Per-module levels
RUST_LOG=passage=debug,passage_adapters=trace passage
```

### Docker

```bash
docker run -d \
  --name passage \
  -p 25565:25565 \
  -v $(pwd)/config:/app/config \
  -e RUST_LOG=debug \
  ghcr.io/scrayosnet/passage:latest
```

### Centralized Logging

Use Docker logging drivers to send logs to Loki, ELK, or other aggregators:

```yaml
# docker-compose.yml
services:
  passage:
    image: ghcr.io/scrayosnet/passage:latest
    logging:
      driver: loki
      options:
        loki-url: "http://loki:3100/loki/api/v1/push"
```

Alternatively, use the OTel `logs` endpoint to send logs directly via OTLP.

---

## Health Checks

Passage doesn't expose a dedicated health endpoint, but you can verify it's running:

```bash
# TCP connection test
nc -zv localhost 25565

# Minecraft status check
pip install mcstatus
mcstatus localhost:25565 status
```

### Kubernetes Probes

```yaml
livenessProbe:
  tcpSocket:
    port: 25565
  initialDelaySeconds: 5
  periodSeconds: 10
readinessProbe:
  tcpSocket:
    port: 25565
  initialDelaySeconds: 2
  periodSeconds: 5
```

---

## Best Practices

- **Enable OpenTelemetry** in production for traces and metrics at minimum
- **Set up Sentry** for error tracking -- it catches issues you won't see in metrics
- **Use `RUST_LOG=info`** as the default log level; switch to `debug` for troubleshooting
- **Monitor adapter response times** -- slow adapters directly impact player connection speed
- **Protect credentials** -- use environment variables for OTel tokens and Sentry DSNs in CI/CD
