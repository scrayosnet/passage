pub mod error;
pub mod stream;

pub(crate) use crate::crypto::error::Error;
use passage_packets::VerifyToken;
use rand::TryRng;
use rand::rand_core::UnwrapErr;
use rand::rngs::SysRng;
use rsa::pkcs8::EncodePublicKey;
use rsa::{Pkcs1v15Encrypt, RsaPrivateKey, RsaPublicKey};
use std::sync::LazyLock;
use tokio::time::Instant;

/// The RSA keypair of the application.
pub static KEY_PAIR: LazyLock<(RsaPrivateKey, RsaPublicKey)> =
    LazyLock::new(|| generate_keypair().expect("failed to generate keypair"));

/// The encoded public key.
pub static ENCODED_PUB: LazyLock<Vec<u8>> =
    LazyLock::new(|| encode_public_key(&KEY_PAIR.1).expect("failed to encode keypair"));

/// A time anchor for generating keep alive packet IDs.
static TIME_ANCHOR: LazyLock<Instant> = LazyLock::new(Instant::now);

/// Generates a random id for a keep alive packet. Just like vanilla servers, it uses a
/// system-dependent time in milliseconds to generate the keep alive ID value.
#[must_use]
pub fn generate_keep_alive() -> u64 {
    TIME_ANCHOR.elapsed().as_millis() as u64
}

/// Generates a new RSA keypair.
fn generate_keypair() -> Result<(RsaPrivateKey, RsaPublicKey), Error> {
    // retrieve a new mutable instance of an OS RNG
    let mut rng = UnwrapErr(SysRng);

    // generate the corresponding key pair
    let private_key = RsaPrivateKey::new(&mut rng, 1024)?;
    let public_key = RsaPublicKey::from(&private_key);

    // return the newly generated key pair
    Ok((private_key, public_key))
}

/// Encodes an RSA public key for the Minecraft protocol.
fn encode_public_key(key: &RsaPublicKey) -> Result<Vec<u8>, Error> {
    Ok(key.to_public_key_der()?.to_vec())
}

/// Encrypts some value with an RSA public key for the Minecraft protocol.
pub fn encrypt(key: &RsaPublicKey, value: &[u8]) -> Result<Vec<u8>, Error> {
    // retrieve a new mutable instance of an OS RNG
    let mut rng = UnwrapErr(SysRng);

    Ok(key.encrypt(&mut rng, Pkcs1v15Encrypt, value)?)
}

/// Decrypts some value with an RSA public key for the Minecraft protocol.
pub fn decrypt(key: &RsaPrivateKey, value: &[u8]) -> Result<Vec<u8>, Error> {
    Ok(key.decrypt(Pkcs1v15Encrypt, value)?)
}

/// Generates a random [`VerifyToken`].
pub fn generate_token() -> Result<VerifyToken, Error> {
    // retrieve a new mutable instance of an OS RNG
    let mut rng = SysRng;

    // populate the random bytes
    let mut data = [0u8; 32];
    rng.try_fill_bytes(&mut data)?;

    Ok(data)
}

/// Checks whether the provided [`VerifyToken`] matches the expected [`VerifyToken`].
pub fn verify_token(expected: VerifyToken, actual: &[u8]) -> bool {
    expected == actual
}

#[cfg(test)]
mod tests {
    use super::*;
    use passage_adapters::authentication::minecraft_hash;
    use std::time::Duration;

    #[test]
    fn create_keypair() {
        generate_keypair().expect("failed to generate keypair");
    }

    #[test]
    fn can_create_keypair() {
        let (_, key) = generate_keypair().expect("failed to generate keypair");
        encode_public_key(&key).expect("failed to encode keypair");
    }

    #[tokio::test(start_paused = true)]
    async fn generate_different_keep_alive() {
        let id1 = generate_keep_alive();
        tokio::time::advance(Duration::new(1, 1)).await;
        let id2 = generate_keep_alive();
        assert_ne!(id1, id2);
    }

    #[test]
    fn generate_different_token() {
        let token1 = generate_token().expect("failed to generate token");
        let token2 = generate_token().expect("failed to generate token");
        assert_ne!(token1, token2);
    }

    #[test]
    fn verify_valid_token() {
        let token = generate_token().expect("failed to generate token");
        assert!(verify_token(token, &token), "token should be valid");
    }

    #[test]
    fn verify_invalid_token_self() {
        let token1 = generate_token().expect("failed to generate token");
        let token2 = generate_token().expect("failed to generate token");
        assert!(!verify_token(token1, &token2), "should be different token");
    }

    #[test]
    fn can_hash() {
        let shared_secret = b"verysecuresecret";
        let (_, key) = generate_keypair().expect("failed to generate keypair");
        let encoded = encode_public_key(&key).expect("failed to encode keypair");
        let _ = minecraft_hash("justchunks", shared_secret, &encoded);
    }
}
