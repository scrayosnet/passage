# Passage Protocol

Contains an opinionated implementation of the Minecraft protocol. It is configured using routes which
match connecting clients and handle them using the route-specific adapters.

A connection progresses through the states `Handshake → Status | Login → Configuration → Transfer`.
The server address from the handshake is matched against the route hostnames (regular expressions,
first match wins), and the matching route supplies the adapters that answer the ping, authenticate the
player, localize disconnect messages and discover the transfer target. Once the transfer packet has
been sent, the connection is dropped and no player state is retained.

## Cookies

Passage uses two cookies for its connections. The first is an **authentication cookie**
(`passage:authentication`) that encodes the authenticated player information. This cookie is signed
using a shared secret such that any server the player is transferred to may skip additional
authentication requests. The second is a **session cookie** (`passage:session`) that holds additional
session information such as the OpenTelemetry tracing context. It is NOT signed and can be easily
tampered with by any client.

| | `AuthCookie` | `SessionCookie` |
|---|---|---|
| Key | `passage:authentication` | `passage:session` |
| Signed | Yes, HMAC-SHA256 over a shared secret | No |
| Contents | Creation timestamp, client address, player name and id, transfer target, profile properties, `extra` | Session id, server address and port, `extra` |
| Validation | Signature, expiry and client IP are checked on every connection | None |

Both types carry an `extra` map for system-specific information; the session cookie's `extra` holds the
W3C Trace Context (`traceparent`) so backend servers can continue the distributed trace.

See the `cookie` module for the exact field definitions, and the
[cookie documentation](https://passage.scrayos.net/advanced/cookies/) for integration guidance on the
backend side.
