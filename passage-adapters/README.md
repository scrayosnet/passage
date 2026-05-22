# Passage Adapters

Passage uses the strategy design pattern to allow for custom logic. These strategies are called adapters.
- *Status Adapter*: Used for getting the server status (ping requests).
- *Authentication Adapter*: Used for authenticating connecting players.
- *Localization Adapter*: Used for localizing any message returned by the protocol or adapters.
- *Discovery Adapter*: Used for discovering potential game server targets to connect players to (player independent).
- *Discovery Adapter Action*: Used for filtering, sorting, and mutating the discovered targets in any way.

The adapters are organized by their primary technology (dependency) into multiple sub-crates. This
crate provides the general interfaces as well as simple implementations. Developers have two options
for adding new adapters, either by creating new adapters in these/new sub-crates or by using the gRPC
adapter.
