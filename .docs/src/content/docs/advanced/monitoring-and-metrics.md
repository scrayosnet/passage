---
title: Monitoring and Metrics
description: Learn how to monitor Passage with OpenTelemetry, Prometheus, and Grafana.
---

This guide shows you how to set up comprehensive monitoring and observability for Passage using OpenTelemetry, Prometheus, Grafana, and Sentry.

## Overview

Passage provides built-in observability through:

- **OpenTelemetry** - Metrics and distributed tracing
- **Sentry** (optional) - Error tracking and reporting
- **Structured Logging** - JSON-formatted logs with context

## OpenTelemetry Integration

Passage natively exports metrics and traces using OpenTelemetry (OTLP over HTTP).

### Configuration

Configure OpenTelemetry in your `config.toml`:

```toml
[otel]
environment = "production"
traces_endpoint = "https://otlp-gateway.example.com/v1/traces"
traces_token = "base64_auth_token"
metrics_endpoint = "https://otlp-gateway.example.com/v1/metrics"
metrics_token = "base64_auth_token"
```

### Grafana Cloud Setup

1. **Get Your Endpoints:**
   - Navigate to **Configuration → Data Sources → OpenTelemetry**
   - Copy the OTLP endpoint URLs

2. **Generate Auth Tokens:**
   ```bash
   # Format: instanceID:token
   echo -n "12345:glc_xxxxx" | base64
   ```

3. **Configure Passage:**
   ```toml
   [otel]
   environment = "production"
   traces_endpoint = "https://otlp-gateway-prod-us-central-0.grafana.net/otlp/v1/traces"
   traces_token = "MTIzNDU6Z2xjX3h4eHh4"
   metrics_endpoint = "https://otlp-gateway-prod-us-central-0.grafana.net/otlp/v1/metrics"
   metrics_token = "MTIzNDU6Z2xjX3h4eHh4"
   ```

### Environment Variables

Override with environment variables:

```bash
export PASSAGE_OTEL_ENVIRONMENT=production
export PASSAGE_OTEL_TRACES_ENDPOINT=https://otlp.example.com/v1/traces
export PASSAGE_OTEL_TRACES_TOKEN=base64_token
export PASSAGE_OTEL_METRICS_ENDPOINT=https://otlp.example.com/v1/metrics
export PASSAGE_OTEL_METRICS_TOKEN=base64_token
```

---

## Metrics

Passage exports the following metrics via OpenTelemetry:

### Connection Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `passage_connections_total` | Counter | Total connection attempts |
| `passage_connections_active` | Gauge | Currently active connections |
| `passage_connections_failed` | Counter | Failed connection attempts |
| `passage_connections_rate_limited` | Counter | Connections blocked by rate limiter |

**Labels:**
- `client_ip` - Client IP address
- `server_address` - Server address connected to
- `protocol_version` - Minecraft protocol version

### Request Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `passage_requests_total` | Counter | Total requests (status pings + logins) |
| `passage_status_requests_total` | Counter | Server list ping requests |
| `passage_login_requests_total` | Counter | Login/join requests |
| `passage_request_duration_seconds` | Histogram | Request processing time |

**Labels:**
- `request_type` - `status` or `login`
- `adapter_type` - Adapter used (`fixed`, `http`, `grpc`, etc.)
- `result` - `success` or `failure`

### Adapter Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `passage_adapter_requests_total` | Counter | Adapter invocations |
| `passage_adapter_errors_total` | Counter | Adapter errors |
| `passage_adapter_duration_seconds` | Histogram | Adapter response time |

**Labels:**
- `adapter_name` - `status`, `discovery`, or `strategy`
- `adapter_type` - Implementation type (`fixed`, `http`, `grpc`, etc.)
- `error_type` - Error category (if applicable)

### Target Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `passage_targets_discovered` | Gauge | Number of discovered backend servers |
| `passage_target_selections_total` | Counter | Target selection operations |
| `passage_target_connections_total` | Counter | Successful target connections |
| `passage_target_connection_failures_total` | Counter | Failed target connections |

**Labels:**
- `target_identifier` - Target server ID
- `target_address` - Target server address

### System Metrics

| Metric | Type | Description |
|--------|------|-------------|
| `passage_uptime_seconds` | Gauge | Uptime in seconds |
| `passage_version_info` | Gauge | Version information (value always 1) |

**Labels:**
- `version` - Passage version

---

## Distributed Tracing

Passage exports distributed traces to help debug performance issues and track request flows.

### Trace Spans

Each connection generates the following spans:

1. **`connection`** - Overall connection lifecycle
   - Attributes: `client_ip`, `server_address`, `protocol_version`, `username`, `user_id`

2. **`status_request`** - Server list ping (if applicable)
   - Attributes: `adapter_type`

3. **`discovery`** - Target discovery
   - Attributes: `adapter_type`, `targets_found`

4. **`strategy`** - Target selection
   - Attributes: `adapter_type`, `selected_target`

5. **`target_connection`** - Backend server connection
   - Attributes: `target_identifier`, `target_address`

### Viewing Traces

In Grafana, navigate to **Explore → Traces** and search by:
- **Service name:** `passage`
- **Operation:** `connection`, `status_request`, `discovery`, etc.
- **Attributes:** `username`, `client_ip`, `target_identifier`

---

## Prometheus Setup

If you're using Prometheus instead of Grafana Cloud, set up an OpenTelemetry Collector:

### OpenTelemetry Collector Configuration

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

  jaeger:
    endpoint: "jaeger:14250"
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
      exporters: [jaeger]
```

### Docker Compose Setup

```yaml
version: '3.8'

services:
  passage:
    image: ghcr.io/scrayosnet/passage:latest
    ports:
      - "25565:25565"
    environment:
      - PASSAGE_OTEL_METRICS_ENDPOINT=http://otel-collector:4318/v1/metrics
      - PASSAGE_OTEL_TRACES_ENDPOINT=http://otel-collector:4318/v1/traces
    depends_on:
      - otel-collector

  otel-collector:
    image: otel/opentelemetry-collector:latest
    command: ["--config=/etc/otel-collector-config.yaml"]
    volumes:
      - ./otel-collector-config.yaml:/etc/otel-collector-config.yaml
    ports:
      - "4318:4318"
      - "8889:8889"

  prometheus:
    image: prom/prometheus:latest
    command:
      - '--config.file=/etc/prometheus/prometheus.yml'
    volumes:
      - ./prometheus.yml:/etc/prometheus/prometheus.yml
    ports:
      - "9090:9090"

  grafana:
    image: grafana/grafana:latest
    ports:
      - "3000:3000"
    environment:
      - GF_SECURITY_ADMIN_PASSWORD=admin
    depends_on:
      - prometheus
```

### Prometheus Configuration

```yaml
# prometheus.yml
global:
  scrape_interval: 15s

scrape_configs:
  - job_name: 'passage'
    static_configs:
      - targets: ['otel-collector:8889']
```

---

## Grafana Dashboards

### Creating a Dashboard

1. **Navigate to Grafana**
2. **Create → Dashboard**
3. **Add Panel**

### Example Queries

#### Connection Rate
```promql
rate(passage_connections_total[5m])
```

#### Active Connections
```promql
passage_connections_active
```

#### Request Latency (p95)
```promql
histogram_quantile(0.95, rate(passage_request_duration_seconds_bucket[5m]))
```

#### Error Rate
```promql
rate(passage_connections_failed[5m])
```

#### Adapter Performance
```promql
histogram_quantile(0.99, rate(passage_adapter_duration_seconds_bucket[5m]))
```

#### Target Distribution
```promql
sum(passage_target_selections_total) by (target_identifier)
```

### Pre-Built Dashboard

A pre-built Grafana dashboard is available in the Passage repository:

```bash
# Import from file
curl -o passage-dashboard.json \
  https://raw.githubusercontent.com/scrayosnet/passage/main/docs/grafana-dashboard.json

# Import in Grafana:
# Dashboard → Import → Upload JSON file
```

---

## Logging

Passage uses structured logging with configurable log levels.

### Log Levels

Set via `RUST_LOG` environment variable:

```bash
# Error only
RUST_LOG=error passage

# Info (default)
RUST_LOG=info passage

# Debug
RUST_LOG=debug passage

# Trace (very verbose)
RUST_LOG=trace passage

# Per-module levels
RUST_LOG=passage=debug,passage::adapter=trace passage
```

### Log Format

Logs are output in JSON format:

```json
{
  "timestamp": "2024-02-05T10:30:45.123Z",
  "level": "INFO",
  "target": "passage::connection",
  "message": "Connection established",
  "client_ip": "192.168.1.100",
  "username": "Steve",
  "user_id": "069a79f4-44e9-4726-a5be-fca90e38aaf5",
  "target": "hub-1"
}
```

### Centralized Logging

#### Loki (Grafana)

```yaml
# docker-compose.yml
services:
  passage:
    image: ghcr.io/scrayosnet/passage:latest
    logging:
      driver: loki
      options:
        loki-url: "http://loki:3100/loki/api/v1/push"
        loki-batch-size: "400"

  loki:
    image: grafana/loki:latest
    ports:
      - "3100:3100"
    volumes:
      - ./loki-config.yaml:/etc/loki/local-config.yaml
```

#### ELK Stack

```yaml
services:
  passage:
    image: ghcr.io/scrayosnet/passage:latest
    logging:
      driver: fluentd
      options:
        fluentd-address: localhost:24224
        tag: passage

  fluentd:
    image: fluent/fluentd:latest
    ports:
      - "24224:24224"
    volumes:
      - ./fluent.conf:/fluentd/etc/fluent.conf
```

---

## Sentry Error Tracking

Sentry provides real-time error tracking and alerting.

### Configuration

```toml
[sentry]
enabled = true
debug = false
address = "https://examplePublicKey@o0.ingest.sentry.io/0"
environment = "production"
```

### Environment Variables

```bash
export PASSAGE_SENTRY_ENABLED=true
export PASSAGE_SENTRY_ADDRESS=https://your-key@sentry.io/project-id
export PASSAGE_SENTRY_ENVIRONMENT=production
```

### What Gets Reported

Sentry captures:
- Panic/crash events
- Adapter errors
- Connection failures
- Configuration errors

Each event includes:
- Stack traces
- Request context (username, IP, target)
- Environment information
- Custom tags and metadata

### Viewing Errors

In Sentry:
1. Navigate to **Issues**
2. Filter by environment (`production`)
3. View stack traces and context
4. Set up alerts for new/recurring errors

---

## Health Checks

Passage doesn't expose a dedicated health check endpoint, but you can monitor health by:

### Connection Test

```bash
# Test if Passage is accepting connections
nc -zv localhost 25565
```

### Minecraft Status Check

```bash
# Using mcstatus tool
pip install mcstatus
mcstatus localhost:25565 status
```

### Kubernetes Liveness Probe

```yaml
apiVersion: v1
kind: Pod
metadata:
  name: passage
spec:
  containers:
  - name: passage
    image: ghcr.io/scrayosnet/passage:latest
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

## Alerting

### Grafana Alerts

Create alerts in Grafana for:

#### High Error Rate
```promql
rate(passage_connections_failed[5m]) > 0.1
```

#### High Latency
```promql
histogram_quantile(0.95, rate(passage_request_duration_seconds_bucket[5m])) > 0.5
```

#### No Active Connections (possible crash)
```promql
passage_connections_active == 0
```

#### Adapter Errors
```promql
rate(passage_adapter_errors_total[5m]) > 0.05
```

### Prometheus Alertmanager

```yaml
# alerting-rules.yml
groups:
  - name: passage
    interval: 30s
    rules:
      - alert: PassageHighErrorRate
        expr: rate(passage_connections_failed[5m]) > 0.1
        for: 2m
        labels:
          severity: warning
        annotations:
          summary: "High error rate detected"
          description: "Error rate is {{ $value }} errors/sec"

      - alert: PassageHighLatency
        expr: histogram_quantile(0.95, rate(passage_request_duration_seconds_bucket[5m])) > 0.5
        for: 5m
        labels:
          severity: warning
        annotations:
          summary: "High latency detected"
          description: "P95 latency is {{ $value }}s"

      - alert: PassageDown
        expr: up{job="passage"} == 0
        for: 1m
        labels:
          severity: critical
        annotations:
          summary: "Passage is down"
          description: "Passage instance has been down for more than 1 minute"
```

---

## Performance Tuning

### Identify Bottlenecks

Use metrics to identify performance issues:

1. **High `passage_adapter_duration_seconds`** → Optimize adapters
2. **High `passage_request_duration_seconds`** → Check adapter performance
3. **High `passage_target_connection_failures_total`** → Backend server issues
4. **High `passage_connections_rate_limited`** → Adjust rate limiter settings

### Adapter Optimization

- Keep adapter response times under 50ms
- Use caching where appropriate
- Implement connection pooling
- Monitor adapter-specific metrics

---

## Best Practices

### Monitoring
- Set up dashboards for key metrics
- Configure alerts for critical issues
- Monitor both Passage and adapters
- Track long-term trends

### Logging
- Use structured logging (JSON)
- Set appropriate log levels
- Aggregate logs centrally
- Include context (username, IP, target)

### Observability
- Enable OpenTelemetry in production
- Use distributed tracing for debugging
- Monitor adapter performance
- Track error rates and latency

### Security
- Protect metrics endpoints
- Secure OTLP credentials
- Monitor for unusual patterns
- Set up security alerts

---

## Troubleshooting

### No Metrics Appearing

1. **Check OTLP endpoints:**
   ```bash
   curl -v $PASSAGE_OTEL_METRICS_ENDPOINT
   ```

2. **Verify authentication:**
   ```bash
   echo $PASSAGE_OTEL_METRICS_TOKEN | base64 -d
   ```

3. **Check logs:**
   ```bash
   RUST_LOG=debug passage
   ```

### High Latency

1. Check adapter metrics:
   ```promql
   passage_adapter_duration_seconds
   ```

2. View traces in Grafana
3. Optimize slow adapters
4. Consider caching

### Missing Traces

1. Ensure traces endpoint is configured
2. Check sample rate (default: 100%)
3. Verify network connectivity
4. Check OpenTelemetry Collector logs

---

## Next Steps

- Learn about [Scaling Strategies](/advanced/scaling/)
- Configure [Custom gRPC Adapters](/advanced/custom-grpc-adapters/)
- Set up [Kubernetes Deployment](/setup/kubernetes/)
- Review [Configuration Reference](/reference/configuration-reference/)
