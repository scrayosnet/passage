---
title: Distributed Tracing
description: How to interpret traces and metrics from Passage in your observability stack, and how to correlate player connections across backend servers.
sidebar:
  order: 4
---

Passage records a **trace** for every player connection — a timeline of everything that happened from the moment a client connected until they were transferred to a backend server. These traces, together with the metrics Passage exports, give you a complete picture of your network's health and player experience.

> **Prerequisite**: Tracing and metrics must be enabled in configuration before anything appears in your observability stack. See [Monitoring and Observability](/advanced/observability) for setup instructions.

## What a Trace Looks Like

Each player connection produces a single trace. In your tracing tool (Grafana Tempo, Jaeger, etc.) it appears as a timeline bar labelled `passage` with a duration equal to the full connection time — typically a few hundred milliseconds for a successful transfer.

The trace is broken into phases that reflect the Minecraft protocol:

| Phase | What it represents |
|-------|--------------------|
| Status check | Responding to a server-list ping (no login involved) |
| Authentication | Verifying the player's identity with the configured auth adapter |
| Configuration | Delivering resource packs and keep-alive packets before the transfer |
| Transfer | Sending the backend address and closing the connection |

Slow or failed connections show up as unusually long or error-marked traces. A spike in authentication duration, for example, points directly to a slow or unreachable auth adapter.

## Service Identity

Every trace and metric Passage emits is tagged with the following labels so you can filter by environment and version in your dashboards:

| Label | Value |
|-------|-------|
| Service name | `passage` |
| Service namespace | `scrayosnet` |
| Service version | Passage version (e.g. `0.3.0`) |
| Environment | Your `otel.environment` config value (e.g. `production`) |

## Metrics Reference

### Connection Metrics

These metrics give you a real-time view of traffic flowing through Passage.

| Metric | What it measures |
|--------|-----------------|
| `listener_requests` | Total incoming connections. The `decision` label splits this into `accepted` (processed normally) and `rejected` (dropped by the rate limiter or a proxy protocol error). |
| `open_connections` | How many player connections are currently being handled. |
| `connection_duration` | How long connections take from start to finish, in seconds. Watch the p95/p99 here — a rise indicates something is slowing down the authentication or discovery phase. |
| `transfer_connections` | Connections grouped by type: `status` (server-list pings), `login` (new player logins), or `transfer` (reconnecting players using a transfer cookie). |
| `rate_limiter_size` | The number of IPs currently tracked by the rate limiter. This should stay small during normal operation and reset itself automatically. A high value may indicate a connection flood. |
| `client_locales` | Distribution of player client languages. Useful for knowing which languages to prioritize for localized disconnect messages. |
| `client_view_distances` | Distribution of view distances reported by clients during login. |

### System Metrics

When `system_observer_interval` is configured, Passage also reports host-level metrics. These help correlate player-facing issues with resource pressure on the host:

| Metric | What it measures |
|--------|-----------------|
| `cpu_usage` | Overall CPU usage of the host (0–100%). |
| `total_memory` / `used_memory` / `free_memory` / `available_memory` | System RAM in bytes. |
| `total_swap` / `used_swap` / `free_swap` | Swap space in bytes. |

## Correlating Traces Across Backend Servers

When Passage creates a session cookie for a player, it embeds the current trace ID into the cookie. This means your backend servers can attach their own spans to the same trace, giving you an unbroken timeline from Passage all the way through your backend network.

The session cookie (`passage:session`) includes an `extra` field containing a `traceparent` value:

```json
{
  "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "server_address": "play.example.com",
  "server_port": 25565,
  "extra": {
    "traceparent": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
  }
}
```

The `traceparent` follows the [W3C Trace Context](https://www.w3.org/TR/trace-context/) standard and contains the trace ID that links the player's entire journey. A backend server that reads this value and passes it to its own OpenTelemetry SDK will appear as a connected child in the same trace — no separate correlation step needed.

This enables scenarios like:
- Viewing a single trace that covers Passage authentication, resource pack delivery, and the player's first few seconds on a lobby server
- Searching your trace backend by trace ID to find every service that touched a specific player connection
- Setting alerts on end-to-end latency rather than per-service latency

The authentication cookie (`passage:authentication`) also has an `extra` field but it does not carry trace context — only the session cookie does.

## Recommended Dashboards and Alerts

### Dashboards to build

- **Traffic overview**: `listener_requests` total rate, split by `decision`. Shows overall throughput and rejection rate over time.
- **Connection latency**: `connection_duration` histogram (p50, p95, p99). The single most useful signal for player-facing performance.
- **Connection types**: `transfer_connections` by `state`. The ratio of `transfer` to `login` shows how effectively auth cookies are working — more transfers means fewer Mojang API calls.
- **Active connections**: `open_connections` as a live gauge. Pair with `connection_duration` to spot overload.
- **Host health**: `cpu_usage` and `used_memory` alongside connection metrics to catch resource-pressure incidents.

### Alerts to consider

- `listener_requests{decision="rejected"}` rate above baseline — possible flood or upstream misconfiguration
- `connection_duration` p99 above ~2 seconds — adapter is slow or unreachable
- `open_connections` growing without new `listener_requests` — connections are stalling

## Sampling in Production

Every player connection produces a trace. For high-traffic networks this volume can be expensive to store. Most tracing backends support **head-based sampling** — configure your OTel Collector or tracing backend to keep only a percentage of traces (5–10% is usually enough for latency analysis). Error traces should always be kept regardless of the sampling rate.

If you use an OTel Collector between Passage and your backend, the probabilistic sampler processor is the simplest option:

```yaml
processors:
  probabilistic_sampler:
    sampling_percentage: 10
```

Passage itself does not perform sampling — all traces are exported and sampling decisions are left to the pipeline.
