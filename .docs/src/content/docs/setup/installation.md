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

The recommended way to deploy Passage on Kubernetes is via the official Helm Chart:

```sh
helm install passage oci://ghcr.io/scrayosnet/helm/passage \
  --version 0.3.0 \
  --namespace passage --create-namespace
```

See the [Kubernetes Guide](/setup/kubernetes/) for the full setup including Agones integration, load balancer
configuration, and production hardening.

## Pelican

A community-maintained [Pelican](https://pelican.dev/) egg is available, created by community member
[@Crayson09](https://github.com/Crayson09). It automates downloading the latest Passage release and
sets up the configuration directory on installation.

Import the egg JSON into your Pelican panel and create a new server from it. The egg will:

- Download the latest Passage binary from GitHub Releases during installation
- Place a copy of the example config at `config/config.yml`
- Start Passage via `./passage` on server startup
- Automatically bind to the port assigned by Pelican (`{{server.allocations.default.port}}`)

**Egg JSON** (import via Pelican panel → Admin → Nests → Import Egg):

```json
{
    "_comment": "DO NOT EDIT: FILE GENERATED AUTOMATICALLY BY PANEL",
    "meta": {
        "version": "PLCN_v3",
        "update_url": null
    },
    "exported_at": "2026-03-03T22:03:41+00:00",
    "name": "Passage",
    "author": "crayson-dev@hotmail.com",
    "uuid": "3a6f6cf3-f44f-450d-8b00-d1104130f1fb",
    "description": "Minecraft Server Transfer Router",
    "image": null,
    "tags": [],
    "features": [],
    "docker_images": {
        "ghcr.io/parkervcp/yolks:rust_latest": "ghcr.io/parkervcp/yolks:rust_latest"
    },
    "file_denylist": [],
    "startup_commands": {
        "Default": "./passage"
    },
    "config": {
        "files": "{\n    \"config/config.yml\": {\n        \"parser\": \"yaml\",\n        \"find\": {\n            \"address\": \"0.0.0.0:{{server.allocations.default.port}}\"\n        }\n    }\n}",
        "startup": "{}",
        "logs": "{}",
        "stop": "^C"
    },
    "scripts": {
        "installation": {
            "script": "#!/bin/bash\n\ncd /mnt/server || exit 1\n\nFILE=\"passage-x86_64-unknown-linux-gnu.tar.gz\"\n\nNEW_RELEASE=$(curl -s https://api.github.com/repos/scrayosnet/passage/releases?per_page=100 \\\n    | jq -r '.[0].tag_name') \n\nURL=\"https://github.com/scrayosnet/passage/releases/download/$NEW_RELEASE/$FILE\"\n\necho \"Downloading Passage binary...\"\nwget -O passage.tar.gz \"${URL}\" || exit 1\n\necho \"Extracting...\"\ntar -xzf passage.tar.gz || exit 1\nrm -rf passage.tar.gz || exit 1\n\nchmod +x passage || exit 1\n\necho \"Creating config folder...\"\nmkdir -p config\nwget -O config/config.yml \"https://raw.githubusercontent.com/scrayosnet/passage/main/config/example.yaml\"\n\necho \"Install complete.\"\nexit 0",
            "container": "ghcr.io/pelican-eggs/installers:debian",
            "entrypoint": "bash"
        }
    },
    "variables": []
}
```

After installation, edit `config/config.yml` via the Pelican file manager to configure your backend
servers before starting the egg.

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
