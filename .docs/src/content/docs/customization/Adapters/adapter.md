---
title: Adapter Overview
description: Understanding Passage's adapter system and how to choose the right adapters for your network.
---

Passage uses three types of pluggable adapters for customizing routing behavior.

## Adapter Types

**Status:** Provides server list information (MOTD, player count, favicon). Options: Fixed, HTTP, gRPC. [Details →](/customization/status-adapters/)

**Discovery:** Lists available backend servers with metadata. Options: Fixed, gRPC, Agones. [Details →](/customization/target-discovery-adapters/)

**Strategy:** Selects which server to send each player to. Options: Fixed, Player Fill, gRPC. [Details →](/customization/target-strategy-adapters/)

## Common Configurations

| Use Case | Status | Discovery | Strategy |
|----------|--------|-----------|----------|
| Single server | Fixed | Fixed | Fixed |
| Multiple servers, fill sequentially | Fixed | Fixed | Player Fill |
| Kubernetes + Agones | HTTP | Agones | Player Fill |
| Custom routing logic | gRPC | gRPC | gRPC |

For complete gRPC implementation examples, see [Custom gRPC Adapters](/advanced/custom-grpc-adapters/).
