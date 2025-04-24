use crate::authentication::Error::InvalidVerifyToken;
use crate::cipher_stream::{Aes128Cfb8Dec, Aes128Cfb8Enc};
use cfb8::cipher::KeyIvInit;
use hmac::{Hmac, Mac};
use num_bigint::BigInt;
use packets::VerifyToken;
use rand::rngs::OsRng;
use rand::{Rng, RngCore};
use rsa::pkcs8::EncodePublicKey;
use rsa::{Pkcs1v15Encrypt, RsaPrivateKey, RsaPublicKey};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use sha2::Sha256;
use std::sync::LazyLock;
use uuid::Uuid;

/// Hmac type, expects 32 Byte hash
pub type HmacSha256 = Hmac<Sha256>;

/// The Mojang host. Used for making authentication requests.
const MOJANG_HOST: &'static str = "https://sessionserver.mojang.com"; // "http://127.0.0.1:8731";

/// The RSA keypair of the application.
pub static KEY_PAIR: LazyLock<(RsaPrivateKey, RsaPublicKey)> =
    LazyLock::new(|| generate_keypair().expect("failed to generate keypair"));

/// The encoded public key.
pub static ENCODED_PUB: LazyLock<Vec<u8>> =
    LazyLock::new(|| encode_public_key(&KEY_PAIR.1).expect("failed to encode keypair"));

/// The shared http client (for mojang requests).
static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .build()
        .expect("failed to create http client")
});

/// The internal error type for all errors related to the authentication and cryptography.
///
/// This includes errors with the expected packets, packet contents or encoding of the exchanged fields. Errors of the
/// underlying data layer (for Byte exchange) are wrapped from the underlying IO errors. Additionally, the internal
/// timeout limits also are covered as errors.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// An error occurred while reading or writing to the underlying byte stream.
    #[error("error reading or writing data: {0}")]
    IllegalRsa(#[from] rsa::Error),
    #[error("could not encode the public key: {0}")]
    EncodingFailed(#[from] rsa::pkcs8::spki::Error),
    #[error("failed to retrieve randomness: {0}")]
    UnavailableRandom(#[from] rand::Error),
    #[error("authentication request failed: {0}")]
    AuthRequestFailed(#[from] reqwest::Error),
    #[error("authentication request failed: {0}")]
    InvalidCipherLength(#[from] cfb8::cipher::InvalidLength),
    /// The received packet is of an invalid length that we cannot process.
    #[error("illegal packet length")]
    IllegalPacketLength,
    #[error("invalid verification token received: {actual:?} (expected: {expected:?})")]
    InvalidVerifyToken {
        expected: VerifyToken,
        actual: Vec<u8>,
    },
}

/// Signs a message with a secret. Returns the signed message. Use [`verify`] to verify and destruct
/// the signed message.
pub fn sign(message: &[u8], secret: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC can take key of any size!");
    mac.update(message);
    let hash = mac.finalize().into_bytes();

    // Build the final binary format (hash is always 32 byte)
    let mut output = Vec::with_capacity(32 + message.len());
    output.extend_from_slice(&hash);
    output.extend_from_slice(message);

    output
}

/// Verifies a signed message with a secret. Returns whether the signature is valid, as well as the
/// inner message. Use [`sign`] to create a signed message.
pub fn verify<'a>(signed: &'a [u8], secret: &[u8]) -> (bool, &'a [u8]) {
    if signed.len() < 32 {
        return (false, b"");
    }

    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC can take key of any size!");
    mac.update(&signed[32..]);
    let ok = mac.verify_slice(&signed[..32]).is_ok();
    (ok, &signed[32..])
}

/// Generates a random id for a keep alive packet.
pub fn generate_keep_alive() -> u64 {
    // retrieve a new mutable instance of an OS RNG
    let mut rng = OsRng;

    // generate random number
    rng.r#gen()
}

/// Generates a new RSA keypair.
fn generate_keypair() -> Result<(RsaPrivateKey, RsaPublicKey), Error> {
    // retrieve a new mutable instance of an OS RNG
    let mut rng = OsRng;

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
    Ok(key.encrypt(&mut OsRng, Pkcs1v15Encrypt, value)?)
}

/// Decrypts some value with an RSA public key for the Minecraft protocol.
pub fn decrypt(key: &RsaPrivateKey, value: &[u8]) -> Result<Vec<u8>, Error> {
    Ok(key.decrypt(Pkcs1v15Encrypt, value)?)
}

/// Generates a random [`VerifyToken`].
pub fn generate_token() -> Result<VerifyToken, Error> {
    // retrieve a new mutable instance of an OS RNG
    let mut rng = OsRng;

    // populate the random bytes
    let mut data = [0u8; 32];
    rng.try_fill_bytes(&mut data)?;

    Ok(data)
}

/// Checks whether the provided [`VerifyToken`] matches the expected [`VerifyToken`].
pub fn verify_token(expected: VerifyToken, actual: &[u8]) -> Result<(), Error> {
    if expected != actual {
        return Err(InvalidVerifyToken {
            expected,
            actual: actual.to_vec(),
        });
    }
    Ok(())
}

/// Creates hash for the Minecraft protocol.
pub fn minecraft_hash(server_id: &str, shared_secret: &[u8], encoded_public: &[u8]) -> String {
    // create a new hasher instance
    let mut hasher = Sha1::new();

    // server id
    hasher.update(server_id);
    // shared secret
    hasher.update(shared_secret);
    // encoded public key
    hasher.update(encoded_public);

    // take the digest and convert it to Minecraft's format
    BigInt::from_signed_bytes_be(&hasher.finalize()).to_str_radix(16)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AuthResponse {
    /// The unique identifier of the Minecraft user profile.
    pub id: Uuid,
    /// The current visual name of the Minecraft user profile.
    pub name: String,
}

/// Makes a authentication request to Mojang.
pub async fn authenticate_mojang(
    username: &str,
    shared_secret: &[u8],
    server_id: &str,
    encoded_public: &[u8],
) -> Result<AuthResponse, Error> {
    // calculate the minecraft hash for this secret, key and username
    let hash = minecraft_hash(server_id, shared_secret, encoded_public);

    // issue a request to Mojang's authentication endpoint
    let url =
        format!("{MOJANG_HOST}/session/minecraft/hasJoined?username={username}&serverId={hash}");
    let response = HTTP_CLIENT.get(&url).send().await?.error_for_status()?;

    // extract the fields of the response
    Ok(response.json().await?)
}

/// Creates a cipher pair for encrypting a TCP stream. The pair is synced for the same shared secret.
pub fn create_ciphers(shared_secret: &[u8]) -> Result<(Aes128Cfb8Enc, Aes128Cfb8Dec), Error> {
    let encoder = Aes128Cfb8Enc::new_from_slices(shared_secret, shared_secret)?;
    let decoder = Aes128Cfb8Dec::new_from_slices(shared_secret, shared_secret)?;
    Ok((encoder, decoder))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_keypair() {
        generate_keypair().expect("failed to generate keypair");
    }

    #[test]
    fn can_create_keypair() {
        let (_, key) = generate_keypair().expect("failed to generate keypair");
        encode_public_key(&key).expect("failed to encode keypair");
    }

    #[test]
    fn sign_verify() {
        let message = b"justchunks";
        let secret = b"secret";

        let signed = sign(message, secret);
        let (ok, verified) = verify(&signed, secret);

        assert!(ok);
        assert_eq!(message, verified);
    }

    #[test]
    fn sign_verify_invalid_secret() {
        let message = b"justchunks";
        let secret1 = b"secret1";
        let secret2 = b"secret2";

        let signed = sign(message, secret1);
        let (ok, _) = verify(&signed, secret2);

        assert!(!ok);
    }

    #[test]
    fn sign_verify_invalid_message() {
        let message = b"justchunks";
        let secret = b"secret";

        let (ok, _) = verify(message, secret);

        assert!(!ok);
    }

    #[test]
    fn generate_different_keep_alive() {
        let id1 = generate_keep_alive();
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
        verify_token(token, &token).expect("token should be valid");
    }

    #[test]
    fn verify_invalid_token_self() {
        let token1 = generate_token().expect("failed to generate token");
        let token2 = generate_token().expect("failed to generate token");
        let Err(_) = verify_token(token1, &token2) else {
            panic!("should be different token")
        };
    }

    #[test]
    fn can_hash() {
        let shared_secret = b"verysecuresecret";
        let (_, key) = generate_keypair().expect("failed to generate keypair");
        let encoded = encode_public_key(&key).expect("failed to encode keypair");
        minecraft_hash("justchunks", shared_secret, &encoded);
    }
}
