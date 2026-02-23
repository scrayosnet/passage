# DNS Discovery Adapter

DNS-based discovery adapter for Passage that resolves backend targets from DNS records.

## Features

- **SRV Records**: Full service discovery with automatic port resolution
- **A/AAAA Records**: Simple hostname-to-IP resolution with configurable default port
- **Auto-refresh**: Periodic DNS queries to discover new targets and remove stale ones
- **Metadata**: Automatically attaches DNS metadata (hostname, priority, weight) to targets

## Usage

Configure the DNS adapter in your `config.toml`:

```toml
[adapters.discovery.dns]
domain = "_minecraft._tcp.example.com"
record_type = "srv"
refresh_interval = 30
```

Or for A/AAAA records:

```toml
[adapters.discovery.dns]
domain = "mc-servers.example.com"
record_type = "a"
port = 25565
refresh_interval = 30
```

## Configuration

- `domain`: The DNS domain to query
- `record_type`: Either `"srv"` or `"a"`
- `port`: Default port (required for A/AAAA records, ignored for SRV)
- `refresh_interval`: DNS query interval in seconds
