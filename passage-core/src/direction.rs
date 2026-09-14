/// Which way a packet travels.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Client to server.
    Serverbound,
    /// Server to client.
    Clientbound,
}

impl Direction {
    /// The opposite direction.
    #[must_use]
    pub const fn flip(self) -> Self {
        match self {
            Direction::Serverbound => Direction::Clientbound,
            Direction::Clientbound => Direction::Serverbound,
        }
    }
}
