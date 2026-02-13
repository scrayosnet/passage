use tokio::time::{Instant, Interval};

#[derive(Debug)]
pub struct KeepAlive<const SIZE: usize> {
    pub packets: [u64; SIZE],
    pub last_sent: Instant,
    pub interval: Interval,
}

impl<const SIZE: usize> KeepAlive<SIZE> {
    pub fn replace(&mut self, from: u64, to: u64) -> bool {
        for entry in &mut self.packets {
            if *entry == from {
                *entry = to;
                return true;
            }
        }
        false
    }
}
