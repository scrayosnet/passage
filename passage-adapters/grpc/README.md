# gRPC Adapters

This crate provides gRPC implementations for all adapters. Each adapter is implemented as a gRPC
client that sends the request to an external gRPC service implementing the interface defined in
`./proto/adapter`. This is the general-purpose escape hatch: any routing, authentication or status
logic that the built-in adapters do not cover can be implemented in any language that speaks gRPC.

All services live in the `scrayosnet.passage.adapter` package:

| Proto file | Service | RPC |
|------------|---------|-----|
| `status.proto` | `Status` | `GetStatus(StatusRequest) -> StatusResponse` |
| `authentication.proto` | `Authentication` | `Authenticate(AuthenticationRequest) -> AuthenticationResponse` |
| `discovery.proto` | `Discovery` | `GetTargets(TargetRequest) -> TargetsResponse` |
| `discovery_action.proto` | `DiscoveryAction` | `Apply(ApplyRequest) -> ApplyResponse` |
| `localization.proto` | `Localization` | `Localize(LocalizationRequest) -> LocalizationResponse` |

Common message types (`Target`, `Address`, `MetaEntry`, `Profile`, `ClientInfo`, `PlayerInfo`) are
shared through `adapter.proto`.

The `Authentication` and `DiscoveryAction` services can reject a connection by returning a `key`
instead of a successful response. That key is resolved through the route's localization adapter and
shown to the player as the disconnect message.

Adapter addresses are full URIs and must include a scheme, for example `http://discovery:50051`. The
client is built without a TLS backend, so plaintext `http://` is the only working scheme — terminate
TLS in a sidecar or service mesh if you need transport encryption.

See the [gRPC protocol reference](https://passage.scrayos.net/reference/grpc-protocol/) for the full
message definitions and the [implementation guide](https://passage.scrayos.net/advanced/grpc-adapters/)
for examples.
