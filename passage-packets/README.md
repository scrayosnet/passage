# Passage Packets

Contains the packet definitions and the binary (de)serialization for the Minecraft: Java Edition
protocol used by Passage.

Packets are grouped by the protocol state they belong to — `handshake`, `status`, `login` and
`configuration` — and each state module separates `clientbound` from `serverbound` packets. The
`codec` module implements the primitive wire types, most notably `VarInt` and `VarLong`, and provides
a `PacketCodec` for cancellation-safe framed reads over a Tokio stream.

Only the packets Passage itself needs are covered, which is everything up to and including the
transfer packet. Play-state packets are intentionally out of scope, because Passage drops the
connection before the play state is ever reached.
