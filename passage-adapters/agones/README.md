# Agones Adapters

This crate provides adapters based on [Agones](https://agones.dev/), an open source, batteries-included,
multiplayer dedicated game server scaling and orchestration platform that can run anywhere Kubernetes
can run.

## Allocation Discovery Adapter

Currently, this crate supports an allocation-based discovery adapter for selecting game servers from
an Agones fleet. The selected game server is directly allocated using Kubernetes CRDs. The allocation
CRD selectors, scheduling, and metadata can be configured using the configuration as JSON. This allows
for full compatability with future selector updates.

The JSON structures also support templating. Currently, this feature is limited to replacing exatcly
matching full string fields (e.g., `{{ .Client.ProtocolVersion }}`). In the future, this may be extended
by using a fully fletched templating engine. We currently support the following variables:
- `{{ .Client.ProtocolVersion }}`
- `{{ .Client.ServerAddress }}`
- `{{ .Client.ServerPort }}`
- `{{ .Client.Address }}`
- `{{ .Request.TraceId }}`

## Tests

The crate provides a fully fletched integration test suite. It uses testcontainers to start a containered
Kubenetes cluster and installs Agones, as well as a test game server, into it. The `crds` directory
contains the official Agones `install.yaml` and example `gameserver.yaml` files.
