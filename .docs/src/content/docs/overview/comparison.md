---
title: Comparison
description: A guide in my new Starlight docs site.
outline: deep
---

Passage aims to replace existing proxy solutions like [Velocity][velocity-docs], [Gate][gate-docs],
[Waterfall][waterfall-docs] and [BungeeCord][bungeecord-docs]. We're convinced that Passage is the ideal way to connect
modern Minecraft networks to the internet and that there are many advantages in using Passage over conventional
Minecraft network proxies.

In this document, we've summarized the biggest pros and some things you may want to consider when using Passage.

:::caution
This comparison may be biased, but we've done our best to give you an accurate overview over the pros and cons of
choosing Passage versus choosing any conventional proxy software.
:::

## Speed

Passage is written in [Rust][rust-docs], one of the best programming languages for performance and reliability. Rust
produces executable binaries that can be run without any virtual machine, framework or overhead. Most Minecraft proxies
are written in Java and therefore rely on the JVM (Java Virtual Machine) which just requires more resources, because of
the periodic garbage collection and the overall nature of being portable.

But even if you compare Passage to binary proxies like [Gate][gate-docs], Passage is still way faster and requires a
fraction of the memory. On top of that

| Feature                                 | BungeeCord | Waterfall | Velocity | Gate | Passage |
|-----------------------------------------|------------|-----------|----------|------|---------|
| Resource efficient                      | ❌          | ✅         | ✅✅       | ✅✅✅  | ✅✅✅✅    |
| Plugins                                 | ❌          | ❌         | ✅        | ❌    | ❌       |
| Secure player information forwarding    | ❌          | ❌         | ✅        | ✅    | ✅       |
| Supporting the latest Minecraft version | ❌          | ❌         | ✅        | ✅    | ✅✅      |
| Actively developed                      | ❓          | ❌         | ✅        | ✅    | ✅       |

## Summary

Advantages of Passage:

* fast (optimized, native code)
* reliable (clean error handling, monitoring, replicas)
* stay online (ha)
* unlimited scalability
* partial ddos protection -> backend servers are more anonymous
* maximum throughput
* native service discovery (kubernetes/etc)
* joining with everything prepared
* performance (rust + no packet rewrite)
* supports Mojang Chat Signing + secure negotiation
* no packet rewriting -> instant version compatibility
* Stateless

Problems with proxies:

* only support versions after update
* introduces transcoding overhead for all packets
* scalability (does not scale well beyond a single instance)
* single point of failure/connection drop on shutdown
* mojang chat signing is complicated/does not work

[rust-docs]: https://www.rust-lang.org/

[bungeecord-docs]: https://github.com/SpigotMC/BungeeCord

[waterfall-docs]: https://github.com/PaperMC/Waterfall

[velocity-docs]: https://github.com/PaperMC/Velocity

[gate-docs]: https://gate.minekube.com/
