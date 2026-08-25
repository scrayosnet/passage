# Passage Driver

This package is a sketch for the Passage Driver, the next-gen Passage implementation. It provides a
general-purpose backbone for handling the Minecraft protocol. Instead of handling packets stricty
sequentially in one big method, it uses hooks and handlers to implement the custom protocol logic.
The backbone only provides the packet parsing and error handling.

> The crate already contains some basic implementations as ideas on how to manage it. This is not
> the final design and should be revised. Additional ideas are in SCRATCH.md.

## Requirements

Given that this is a next-gen implementation, it should try to be as efficient as possible. It should
also fix issues with the current implementation.
- Packets should not hardcode the packet ID and codec.
  - Instead, they should be able to react to the current protocol version.
  - Passage should be backward-compatible with all protocol versions without duplicating the code.
    - Changes include packet ID changes, adding or removing packets, adding or removing fields from packets
  - Do not create a new packet type for every protocol version, instead use optional fields or similar.
- The backbone (currently called "driver") should only implement the necessary functions (e.g., parsing, error handling, ticks for keep alive packets, and shutdown)
  - This should follow state-of-the-art frameworks like Ktor, Axum, or Tonic
- Ontop of the backbone, a basic server and client implementation should be provided.
  - It uses hooks of its own to implement and overwrite logic
- Ontop of the server, the Passage router with its adapter can be implemented (or configured).
- The application should keep the telemetry and even expand upon it by having better traces
