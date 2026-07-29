# DNS Adapters

This crate provides adapters based on DNS. It resolves backend targets for Passage from DNS records.

## DNS Discovery Adapter

Currently, this crate supports a DNS-based discovery adapter for discovering targets based on SRV or
A/AAAA records. The discovery is implemented as an async cache that is periodically refreshed. This
refresh is fully disconnected from the target selection such that refreshes do not block the player
login flow.

- **SRV Records**: Full service discovery with automatic port resolution
- **A/AAAA Records**: Simple hostname-to-IP resolution with configurable default port

The discovered targets get the DNS metadata attached. Currently, this includes the `priority` and
`weight` for SRV records, as well as the `domain` for both record types.
