---
title: Customization Overview
description: Learn how to customize Passage for your Minecraft network's specific needs.
---

Passage is designed to be highly customizable while maintaining simplicity. This page provides an overview of customization options and helps you understand which customizations are right for your network.

## Philosophy: Start Simple, Scale Complexity

Passage follows a progressive enhancement philosophy:

1. **Start with fixed adapters** - Get running quickly with static configuration
2. **Add dynamic elements** - Introduce HTTP or Agones adapters as needed
3. **Implement custom logic** - Use gRPC adapters for complex requirements
4. **Monitor and optimize** - Add observability and fine-tune performance

You don't need to use all features at once. Many successful networks run with just fixed adapters.

## What Can Be Customized?

### Core Settings
- **Network binding**: Which address and port to listen on
- **Timeouts**: Connection timeout durations
- **Rate limiting**: Connection flood protection
- **PROXY protocol**: Support for load balancers

[→ Configuration Reference](/customization/config/)

### Status Information
Control what players see in their server list:
- Server name (MOTD)
- Player counts
- Favicon
- Version compatibility

**Available options:**
- Static configuration
- Dynamic HTTP endpoint
- Custom gRPC service

[→ Status Adapters Guide](/customization/status-adapters/)

### Server Discovery
Determine which backend servers are available:
- Static server list
- Dynamic from external service
- Auto-discovery from Kubernetes/Agones

**Available options:**
- Fixed configuration
- gRPC service
- Agones GameServer discovery

[→ Target Discovery Guide](/customization/target-discovery-adapters/)

### Routing Strategy
Choose how players are distributed across servers:
- First available
- Fill servers sequentially
- Custom logic (region-based, skill-based, etc.)

**Available options:**
- Fixed (first server)
- Player Fill (consolidate players)
- gRPC (custom logic)

[→ Target Strategy Guide](/customization/target-strategy-adapters/)

### Observability
Monitor Passage's performance and health:
- OpenTelemetry metrics and traces
- Sentry error tracking
- Structured logging

[→ Monitoring Guide](/advanced/monitoring-and-metrics/)

### Localization
Customize disconnect messages in multiple languages:
- Default locale selection
- Per-locale message customization
- Parameter substitution support

[→ Localization Guide](/advanced/localization/)

## Common Customization Scenarios

### Scenario 1: Small Static Network

**Need:** 2-3 fixed servers, simple routing

**Solution:**
```toml
[status]
adapter = "fixed"

[target_discovery]
adapter = "fixed"
[[target_discovery.fixed.targets]]
identifier = "lobby"
address = "10.0.0.10:25565"

[target_strategy]
adapter = "fixed"
```

**Complexity:** None - pure configuration

---

### Scenario 2: Multiple Lobbies with Fill Strategy

**Need:** 5 lobby servers, consolidate players for better experience

**Solution:**
```toml
[status]
adapter = "fixed"

[target_discovery]
adapter = "fixed"
# ... list all 5 lobbies with player counts in metadata

[target_strategy]
adapter = "player_fill"
[target_strategy.player_fill]
field = "players"
max_players = 50
```

**Complexity:** Low - requires updating player counts in metadata

---

### Scenario 3: Kubernetes with Auto-Scaling

**Need:** Cloud deployment with dynamic game servers

**Solution:**
```toml
[status]
adapter = "http"
[status.http]
address = "http://status-service/status"
cache_duration = 5

[target_discovery]
adapter = "agones"
[target_discovery.agones]
namespace = "minecraft"

[target_strategy]
adapter = "player_fill"
[target_strategy.player_fill]
field = "players"
max_players = 50
```

**Complexity:** Medium - requires Kubernetes and Agones setup

---

### Scenario 4: Multi-Region Routing

**Need:** Route players to nearest regional server based on IP

**Solution:**
```toml
[status]
adapter = "grpc"

[target_discovery]
adapter = "grpc"

[target_strategy]
adapter = "grpc"
# Custom gRPC service implements geo-IP lookup
```

**Complexity:** High - requires custom gRPC services

[→ Custom gRPC Adapters Guide](/advanced/custom-grpc-adapters/)

## Customization Roadmap

### Phase 1: Getting Started
1. Install Passage with default configuration
2. Configure one backend server
3. Test connection flow
4. Enable rate limiting

### Phase 2: Basic Customization
1. Customize status (MOTD, favicon)
2. Add multiple backend servers
3. Configure player fill strategy
4. Set up basic monitoring

### Phase 3: Dynamic Elements
1. Switch to HTTP status adapter (if needed)
2. Integrate with Kubernetes/Agones (if applicable)
3. Implement custom target filters
4. Add localization for your languages

### Phase 4: Advanced Features
1. Implement custom gRPC adapters
2. Add complex routing logic
3. Integrate with existing infrastructure
4. Optimize for high-scale deployment

## Best Practices

### Configuration Management

✅ **Do:**
- Use version control for config files (except secrets)
- Start with minimal config and add as needed
- Document your customizations
- Test configuration changes in staging first

❌ **Don't:**
- Commit secrets to version control
- Over-configure before understanding needs
- Change too many things at once
- Skip testing after config changes

### Adapter Selection

✅ **Do:**
- Choose the simplest adapter that meets your needs
- Keep adapters fast (<50ms response time)
- Monitor adapter performance
- Implement fallback behavior

❌ **Don't:**
- Use gRPC adapters unless you need custom logic
- Make slow API calls in adapter implementations
- Forget to cache expensive operations
- Block on I/O in adapter code

### Performance Optimization

✅ **Do:**
- Enable rate limiting in production
- Use appropriate cache durations
- Monitor connection latency
- Scale horizontally when needed

❌ **Don't:**
- Run without rate limiting
- Cache status for too long (>30 seconds)
- Ignore performance metrics
- Add unnecessary complexity

## Customization Checklist

Before deploying your customized Passage:

- [ ] Configuration is complete and valid
- [ ] All adapters respond quickly (<50ms)
- [ ] Rate limiting is enabled and tuned
- [ ] Observability is configured
- [ ] Localization covers your player base
- [ ] Failover behavior is tested
- [ ] Documentation is updated
- [ ] Staging environment tested successfully
- [ ] Rollback plan is in place
- [ ] Monitoring alerts are configured

## Getting Help

### Documentation Resources
- [Configuration Reference](/customization/config/) - All config options
- [Adapter Overview](/customization/adapter/) - Understanding adapters
- [Advanced Topics](/advanced/custom-grpc-adapters/) - Complex customizations

### Community Support
- [GitHub Discussions](https://github.com/scrayosnet/passage/discussions) - Ask questions
- [Discord Server](https://discord.gg/xZ4wbuuKZf) - Real-time help
- [GitHub Issues](https://github.com/scrayosnet/passage/issues) - Report bugs

### Professional Support
For enterprise deployments or custom development:
- Contact the maintainers via GitHub
- Consider sponsoring the project for priority support
