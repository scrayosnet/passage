---
title: Installation
description: Install Passage on your server using Docker, binary, or Kubernetes.
---

Installation methods for Passage. For a complete setup guide, see [Getting Started](/setup/).

## Docker

```bash
docker run -d \
  --name passage \
  -p 25565:25565 \
  -v ./config:/app/config \
  ghcr.io/scrayosnet/passage:latest
```

Or with Docker Compose:

```yaml
services:
  passage:
    image: ghcr.io/scrayosnet/passage:latest
    ports:
      - "25565:25565"
    volumes:
      - ./config:/app/config
    restart: unless-stopped
```

## Binary

Download from [GitHub Releases](https://github.com/scrayosnet/passage/releases):

```bash
# Linux
wget https://github.com/scrayosnet/passage/releases/latest/download/passage-linux-x86_64
chmod +x passage-linux-x86_64
mv passage-linux-x86_64 /usr/local/bin/passage

# macOS (Apple Silicon)
wget https://github.com/scrayosnet/passage/releases/latest/download/passage-darwin-aarch64

# macOS (Intel)
wget https://github.com/scrayosnet/passage/releases/latest/download/passage-darwin-x86_64
```

Or build from source (requires Rust 1.88+):

```bash
git clone https://github.com/scrayosnet/passage.git
cd passage
cargo build --release
# Binary at target/release/passage
```

## Kubernetes

See the [Kubernetes Guide](/setup/kubernetes/) for detailed setup instructions.

## Systemd Service (Linux)

```ini
# /etc/systemd/system/passage.service
[Unit]
Description=Passage Minecraft Transfer Router
After=network.target

[Service]
Type=simple
User=minecraft
ExecStart=/usr/local/bin/passage
WorkingDirectory=/opt/passage
Restart=always
Environment="CONFIG_FILE=/opt/passage/config/config.toml"

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now passage
```
