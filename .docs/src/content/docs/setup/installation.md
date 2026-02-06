---
title: Installation
description: Install Passage on your server using Docker, binary, or Kubernetes.
---

Passage can be installed in multiple ways depending on your deployment environment. Choose the method that best suits your infrastructure.

## Prerequisites

- A server running Linux, macOS, or Windows
- Minecraft backend servers configured and accessible
- (Optional) Docker or Kubernetes for containerized deployments

## Quick Start with Docker

The fastest way to get started is using Docker:

```bash
docker run -d \
  --name passage \
  -p 25565:25565 \
  -v ./config:/app/config \
  ghcr.io/scrayosnet/passage:latest
```

This will:
- Pull the latest Passage image from GitHub Container Registry
- Expose port 25565 for Minecraft connections
- Mount a local `config` directory for configuration files

### Docker Compose

For production deployments, use Docker Compose:

```yaml
# docker-compose.yml
version: '3.8'

services:
  passage:
    image: ghcr.io/scrayosnet/passage:latest
    container_name: passage
    ports:
      - "25565:25565"
    volumes:
      - ./config:/app/config
    environment:
      - PASSAGE_ADDRESS=0.0.0.0:25565
      - PASSAGE_TARGET_DISCOVERY_ADAPTER=fixed
    restart: unless-stopped
```

Start with:
```bash
docker-compose up -d
```

## Install from Binary

### Download Pre-built Binary

Download the latest release for your platform:

```bash
# Linux x86_64
wget https://github.com/scrayosnet/passage/releases/latest/download/passage-linux-x86_64

# macOS (Apple Silicon)
wget https://github.com/scrayosnet/passage/releases/latest/download/passage-darwin-aarch64

# macOS (Intel)
wget https://github.com/scrayosnet/passage/releases/latest/download/passage-darwin-x86_64
```

Make it executable:
```bash
chmod +x passage-*
mv passage-* /usr/local/bin/passage
```

### Build from Source

Requires Rust 1.88.0 or later:

```bash
# Clone the repository
git clone https://github.com/scrayosnet/passage.git
cd passage

# Build with default features (Agones, gRPC, Sentry)
cargo build --release

# Build with specific features only
cargo build --release --no-default-features --features grpc

# Install to system
cargo install --path .
```

The binary will be available at `target/release/passage` or in your Cargo bin directory.

## Kubernetes Deployment

For Kubernetes deployments, see our detailed [Kubernetes Guide](/setup/kubernetes/).

### Quick Deploy with Kubectl

```bash
# Create namespace
kubectl create namespace passage

# Create basic deployment
kubectl apply -f - <<EOF
apiVersion: apps/v1
kind: Deployment
metadata:
  name: passage
  namespace: passage
spec:
  replicas: 2
  selector:
    matchLabels:
      app: passage
  template:
    metadata:
      labels:
        app: passage
    spec:
      containers:
      - name: passage
        image: ghcr.io/scrayosnet/passage:latest
        ports:
        - containerPort: 25565
          protocol: TCP
        env:
        - name: PASSAGE_ADDRESS
          value: "0.0.0.0:25565"
---
apiVersion: v1
kind: Service
metadata:
  name: passage
  namespace: passage
spec:
  type: LoadBalancer
  ports:
  - port: 25565
    targetPort: 25565
    protocol: TCP
  selector:
    app: passage
EOF
```

## Configuration

After installation, you need to configure Passage. Create a configuration file at `config/config.toml`:

```toml
# config/config.toml

# Bind address for Minecraft connections
address = "0.0.0.0:25565"

# Connection timeout in seconds
timeout = 120

# Status adapter configuration
[status]
adapter = "fixed"

[status.fixed]
name = "My Minecraft Network"
description = "\"Welcome to my server!\""
favicon = "data:image/png;base64,..."

# Target discovery configuration
[target_discovery]
adapter = "fixed"

[[target_discovery.fixed.targets]]
identifier = "hub-1"
address = "10.0.1.10:25565"
meta = { type = "hub" }

[[target_discovery.fixed.targets]]
identifier = "survival-1"
address = "10.0.2.10:25565"
meta = { type = "survival" }

# Target strategy configuration
[target_strategy]
adapter = "fixed"
```

See the [Configuration Guide](/customization/config/) for all available options.

## Running Passage

### Using the Binary

```bash
# Run with default config location
passage

# Specify custom config file
CONFIG_FILE=/path/to/config passage

# Use environment variables
PASSAGE_ADDRESS=0.0.0.0:25566 passage
```

### Using Docker

```bash
docker run -d \
  --name passage \
  -p 25565:25565 \
  -v ./config:/app/config \
  -e PASSAGE_ADDRESS=0.0.0.0:25565 \
  ghcr.io/scrayosnet/passage:latest
```

### Using Systemd (Linux)

Create a systemd service file:

```ini
# /etc/systemd/system/passage.service
[Unit]
Description=Passage Minecraft Transfer Router
After=network.target

[Service]
Type=simple
User=minecraft
Group=minecraft
ExecStart=/usr/local/bin/passage
WorkingDirectory=/opt/passage
Restart=always
RestartSec=10

# Environment variables (optional)
Environment="PASSAGE_ADDRESS=0.0.0.0:25565"
Environment="CONFIG_FILE=/opt/passage/config/config.toml"

[Install]
WantedBy=multi-user.target
```

Enable and start:
```bash
sudo systemctl daemon-reload
sudo systemctl enable passage
sudo systemctl start passage
```

## Verifying Installation

Check if Passage is running:

```bash
# Check if port is listening
netstat -tuln | grep 25565

# Or using ss
ss -tuln | grep 25565
```

Test connection:
```bash
# Add server to Minecraft client
# Server Address: localhost:25565 (or your server IP)
# You should see the configured MOTD and player count
```

Check logs:
```bash
# Docker
docker logs passage

# Systemd
journalctl -u passage -f

# Binary (if using stdout)
./passage 2>&1 | tee passage.log
```

## Updating Passage

### Docker

```bash
docker pull ghcr.io/scrayosnet/passage:latest
docker stop passage
docker rm passage
# Re-run docker run command from Quick Start
```

### Binary

Download the new version and replace the old binary:
```bash
wget https://github.com/scrayosnet/passage/releases/latest/download/passage-linux-x86_64
chmod +x passage-linux-x86_64
sudo mv passage-linux-x86_64 /usr/local/bin/passage
sudo systemctl restart passage
```

### Kubernetes

```bash
kubectl rollout restart deployment/passage -n passage
```

## Troubleshooting

### Port Already in Use

If port 25565 is already in use:

```bash
# Find what's using the port
sudo lsof -i :25565

# Change Passage's port
PASSAGE_ADDRESS=0.0.0.0:25566 passage
```

### Permission Denied

On Linux, binding to ports below 1024 requires root or capabilities:

```bash
# Option 1: Run as root (not recommended)
sudo passage

# Option 2: Give capability to binary
sudo setcap 'cap_net_bind_service=+ep' /usr/local/bin/passage

# Option 3: Use a port above 1024 and forward with iptables
sudo iptables -t nat -A PREROUTING -p tcp --dport 25565 -j REDIRECT --to-port 25566
```

### Connection Refused

Ensure:
1. Passage is running: `systemctl status passage`
2. Firewall allows connections: `sudo ufw allow 25565/tcp`
3. Backend servers are accessible from Passage

### See Detailed Logs

Enable trace logging:

```bash
RUST_LOG=trace passage
```

Or configure log level in environment:
```bash
export RUST_LOG=passage=debug,info
passage
```

## Next Steps

- Follow the [Getting Started Guide](/setup/getting-started/) for initial configuration
- Learn about [Configuration Options](/customization/config/)
- Set up [Kubernetes Integration](/setup/kubernetes/) for cloud deployments
- Configure [Monitoring](/advanced/monitoring-and-metrics/) for production
