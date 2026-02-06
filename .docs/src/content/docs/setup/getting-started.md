---
title: Getting Started
description: Get up and running with Passage in 5 minutes - a beginner-friendly guide.
---

This guide will walk you through setting up Passage for the first time. By the end, you'll have a working Minecraft network entry point that authenticates and routes players to backend servers.

## Prerequisites

Before you begin, make sure you have:

- A Minecraft backend server running version **1.20.5 or higher**
- [Docker](https://docs.docker.com/get-docker/) installed (recommended), or
- A Linux/macOS/Windows server for binary deployment
- Basic knowledge of TOML configuration files

:::tip[Already Have Docker?]
You can be up and running in under 5 minutes with our Quick Start below!
:::

## Quick Start (5 Minutes)

### 1. Create Configuration Directory

```bash
mkdir -p passage/config
cd passage
```

### 2. Create Configuration File

Create `config/config.toml` with your backend server details:

```toml
# config/config.toml

# Bind address for player connections
address = "0.0.0.0:25565"

# Connection timeout in seconds
timeout = 120

# Server status configuration
[status]
adapter = "fixed"

[status.fixed]
name = "My Minecraft Network"
description = "\"Welcome to my server!\""

# Target discovery - where to send players
[target_discovery]
adapter = "fixed"

[[target_discovery.fixed.targets]]
identifier = "lobby-1"
address = "10.0.1.10:25565"  # Replace with your backend server IP
meta = { type = "lobby" }

# Target strategy - how to select servers
[target_strategy]
adapter = "fixed"

# Localization
[localization]
default_locale = "en_US"
```

:::caution[Important: Update Backend Address]
Replace `10.0.1.10:25565` with your actual Minecraft server's IP address and port. If running locally, use `host.docker.internal:25565` (Docker Desktop) or `172.17.0.1:25565` (Linux).
:::

### 3. Start Passage

```bash
docker run -d \
  --name passage \
  -p 25565:25565 \
  -v $(pwd)/config:/app/config \
  ghcr.io/scrayosnet/passage:latest
```

### 4. Test Connection

Open Minecraft (version 1.20.5+) and add a server:
- **Server Address**: `localhost:25565` (or your server IP)
- **Server Name**: My Minecraft Network

You should see your configured MOTD and be able to connect!

### 5. Check Logs

```bash
# View real-time logs
docker logs -f passage

# Expected output:
# [INFO] Passage starting on 0.0.0.0:25565
# [INFO] Status adapter: fixed
# [INFO] Target discovery adapter: fixed
# [INFO] Target strategy adapter: fixed
```

🎉 **Congratulations!** You now have Passage running and routing players to your backend server.

## Understanding the Configuration

Let's break down what each section does:

### Core Settings

```toml
address = "0.0.0.0:25565"
timeout = 120
```

- **address**: The IP and port Passage listens on for player connections
  - `0.0.0.0` means accept connections from any network interface
  - `25565` is the default Minecraft port
- **timeout**: How long (in seconds) to wait for a player response before disconnecting

### Status Configuration

```toml
[status]
adapter = "fixed"

[status.fixed]
name = "My Minecraft Network"
description = "\"Welcome to my server!\""
```

The **Status Adapter** provides the server list information (MOTD) that players see:
- `name`: Server name in the multiplayer list
- `description`: MOTD message (must be JSON text or quoted string)

:::note[MOTD Formatting]
The description uses JSON text format. For colored text:
```toml
description = "{\"text\":\"Welcome!\",\"color\":\"gold\"}"
```
:::

### Target Discovery Configuration

```toml
[target_discovery]
adapter = "fixed"

[[target_discovery.fixed.targets]]
identifier = "lobby-1"
address = "10.0.1.10:25565"
meta = { type = "lobby" }
```

**Target Discovery** finds available backend servers:
- `identifier`: Unique name for this server
- `address`: Network address (IP:port) to transfer players to
- `meta`: Custom key-value data (used by strategy adapters)

You can add multiple targets:

```toml
[[target_discovery.fixed.targets]]
identifier = "lobby-1"
address = "10.0.1.10:25565"
meta = { type = "lobby", players = "15" }

[[target_discovery.fixed.targets]]
identifier = "survival-1"
address = "10.0.2.10:25565"
meta = { type = "survival", players = "42" }
```

### Target Strategy Configuration

```toml
[target_strategy]
adapter = "fixed"
```

**Target Strategy** decides which server to send each player to:
- `fixed`: Always chooses the first available target (simple, predictable)

For more advanced routing (like load balancing), see [Player Fill Strategy](/customization/target-strategy-adapters/#player-fill-strategy-adapter).

### Localization Configuration

```toml
[localization]
default_locale = "en_US"
```

Controls the language for disconnect messages and other text sent to players.

## Testing Your Setup

### Test 1: Status Request

Before connecting, verify the status endpoint:

```bash
# If you have mcrcon or mcstatus installed
mcstatus localhost:25565 status

# Expected output:
# version: v1.21.4 (protocol 769)
# description: Welcome to my server!
# players: 0/100
```

### Test 2: Successful Connection

1. Open Minecraft (1.20.5+)
2. Add server: `localhost:25565`
3. Join the server
4. You should be transferred to your backend server

### Test 3: Check Logs

Watch Passage logs during connection:

```bash
docker logs -f passage
```

Expected log flow:
```
[INFO] Accepted connection from 192.168.1.100:54321
[INFO] Player "Steve" authenticated with Mojang
[INFO] Selected target: lobby-1 (10.0.1.10:25565)
[INFO] Transferred player "Steve" to lobby-1
[INFO] Connection closed
```

## Common First-Time Issues

### Issue: "Can't connect to server"

**Symptoms**: Connection times out or "Connection refused"

**Solutions**:
1. Check Passage is running: `docker ps | grep passage`
2. Verify port is listening: `netstat -tuln | grep 25565`
3. Check firewall allows port 25565: `sudo ufw allow 25565/tcp`
4. Ensure backend server is accessible from Passage's network

### Issue: "Outdated client!" or "Outdated server!"

**Symptoms**: Minecraft shows version mismatch error

**Solutions**:
- Ensure your Minecraft client is **1.20.5 or higher**
- Verify your backend server supports the client version
- Check `preferred_version` in config matches your server

### Issue: Player connects but immediately disconnects

**Symptoms**: Brief connection then kicked with "No available server"

**Solutions**:
1. Verify backend server address is correct in `config.toml`
2. Test backend connectivity from Passage:
   ```bash
   docker exec passage ping 10.0.1.10  # Replace with your IP
   ```
3. Ensure backend server is running and accepting connections
4. Check backend server logs for connection attempts

### Issue: Wrong MOTD or server list info

**Symptoms**: Server shows wrong name/description in multiplayer list

**Solutions**:
1. Verify `config.toml` syntax (especially quoted JSON strings)
2. Restart Passage after config changes:
   ```bash
   docker restart passage
   ```
3. Refresh server list in Minecraft (right-click server → Refresh)

### Issue: Docker can't connect to backend on localhost

**Symptoms**: Backend on same machine isn't reachable

**Solutions**:
- **Docker Desktop (Mac/Windows)**: Use `host.docker.internal` instead of `localhost`
  ```toml
  address = "host.docker.internal:25565"
  ```
- **Linux**: Use Docker host IP (usually `172.17.0.1`)
  ```toml
  address = "172.17.0.1:25565"
  ```
- Or use `--network host` when running Docker (Linux only)

## Next Steps

Now that you have Passage running, explore more advanced features:

### 1. **Add Multiple Servers**

Configure multiple backend servers and implement load balancing:
- [Player Fill Strategy](/customization/target-strategy-adapters/#player-fill-strategy-adapter) - Fill servers sequentially
- [Fixed Strategy](/customization/target-strategy-adapters/#fixed-strategy-adapter) - Simple routing

### 2. **Customize Server Status**

Enhance your server list appearance:
- Add a custom favicon
- Use colored MOTD with JSON formatting
- Display dynamic player counts
- [Status Adapters Guide](/customization/status-adapters/)

### 3. **Dynamic Server Discovery**

Replace static configuration with dynamic discovery:
- [gRPC Adapters](/customization/target-discovery-adapters/#grpc-discovery-adapter) - Custom service integration
- [Agones Discovery](/customization/target-discovery-adapters/#agones-discovery-adapter) - Kubernetes game servers

### 4. **Production Deployment**

Prepare for production use:
- [Kubernetes Deployment](/setup/kubernetes/) - High availability and auto-scaling
- [Configuration Best Practices](/customization/config/) - Security and optimization
- [Monitoring and Metrics](/advanced/monitoring-and-metrics/) - OpenTelemetry integration

### 5. **Advanced Customization**

Build custom routing logic:
- [Custom gRPC Adapters](/advanced/custom-grpc-adapters/) - Implement your own adapters
- [Authentication Cookies](/overview/authentication-and-encryption/#authentication-cookies) - Skip re-authentication
- [Rate Limiting](/customization/config/#rate_limiter) - Prevent connection floods

## Configuration Examples

### Example 1: Multiple Lobby Servers (Load Balancing)

```toml
address = "0.0.0.0:25565"
timeout = 120

[status]
adapter = "fixed"
[status.fixed]
name = "BigNetwork"
description = "\"500 players online!\""

[target_discovery]
adapter = "fixed"

[[target_discovery.fixed.targets]]
identifier = "lobby-1"
address = "10.0.1.10:25565"
meta = { type = "lobby", players = "45" }

[[target_discovery.fixed.targets]]
identifier = "lobby-2"
address = "10.0.1.11:25565"
meta = { type = "lobby", players = "38" }

[[target_discovery.fixed.targets]]
identifier = "lobby-3"
address = "10.0.1.12:25565"
meta = { type = "lobby", players = "52" }

[target_strategy]
adapter = "player_fill"
[target_strategy.player_fill]
field = "players"
max_players = 50
```

This configuration will fill lobbies to 50 players before moving to the next one.

### Example 2: Region-Based Routing

For region-based routing with custom logic, you'll need a gRPC adapter. See [Custom gRPC Adapters](/advanced/custom-grpc-adapters/) for implementation details.

### Example 3: Kubernetes with Agones

```toml
address = "0.0.0.0:25565"
timeout = 120

[status]
adapter = "http"
[status.http]
address = "http://status-service.minecraft/status"
cache_duration = 5

[target_discovery]
adapter = "agones"
[target_discovery.agones]
namespace = "minecraft"
label_selector = "game=minecraft,type=lobby"

[target_strategy]
adapter = "player_fill"
[target_strategy.player_fill]
field = "players"
max_players = 50
```

This discovers Minecraft lobbies running as Agones GameServers and fills them sequentially. See [Kubernetes Guide](/setup/kubernetes/) for full setup.

## Troubleshooting Checklist

Before asking for help, verify:

- ✅ Minecraft client version is **1.20.5 or higher**
- ✅ Backend server version is **1.20.5 or higher**
- ✅ Backend server is running and accessible
- ✅ Port 25565 is not blocked by firewall
- ✅ Configuration file is valid TOML
- ✅ Backend address is correct (not localhost if using Docker)
- ✅ Passage logs show no errors (`docker logs passage`)

## Getting Help

If you encounter issues:

1. **Check logs**: `docker logs passage` often reveals the problem
2. **Enable debug logging**: Set `RUST_LOG=debug` environment variable
   ```bash
   docker run -d \
     --name passage \
     -p 25565:25565 \
     -v $(pwd)/config:/app/config \
     -e RUST_LOG=debug \
     ghcr.io/scrayosnet/passage:latest
   ```
3. **Review documentation**: Many common issues are covered in specific guides
4. **Community support**: Visit our [GitHub Discussions](https://github.com/scrayosnet/passage/discussions)
5. **Bug reports**: File issues at [GitHub Issues](https://github.com/scrayosnet/passage/issues)

## Summary

You've learned:
- ✅ How to install and run Passage with Docker
- ✅ Basic configuration structure (status, discovery, strategy)
- ✅ How to connect Minecraft clients to Passage
- ✅ Common troubleshooting steps
- ✅ Next steps for advanced features

Passage is now routing players to your backend servers with minimal overhead and maximum performance!

## Further Reading

- [Architecture](/overview/architecture/) - Understand how Passage works
- [Configuration Reference](/customization/config/) - All configuration options
- [Adapter System](/customization/adapter/) - Deep dive into adapters
- [Comparison with Proxies](/overview/comparison/) - Why choose Passage
