pub mod codec;
pub mod connection;
mod direction;
pub mod router;
mod packet;
mod phase;
mod version;
pub mod wire;
pub mod driver;

pub fn add(left: u64, right: u64) -> u64 {
    left + right
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let result = add(2, 2);
        assert_eq!(result, 4);
    }
}
