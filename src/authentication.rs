use crate::authentication::Error::InvalidVerifyToken;
use crate::cipher_stream::{Aes128Cfb8Dec, Aes128Cfb8Enc};
use cfb8::cipher::KeyIvInit;
use hmac::{Hmac, Mac};
use lazy_static::lazy_static;
use num_bigint::BigInt;
use packets::VerifyToken;
use rand::rngs::OsRng;
use rand::{Rng, RngCore};
use rsa::pkcs8::EncodePublicKey;
use rsa::{Pkcs1v15Encrypt, RsaPrivateKey, RsaPublicKey};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use sha2::Sha256;
use uuid::Uuid;

lazy_static! {
    /// The RSA keypair of the application.
    pub(crate) static ref KEY_PAIR: (RsaPrivateKey, RsaPublicKey) = generate_keypair().expect("failed to generate keypair");

    /// The encoded public key.
    pub(crate) static ref ENCODED_PUB: Vec<u8> = encode_public_key(&KEY_PAIR.1).expect("failed to encode keypair");

    /// The shared http client (for mojang requests).
    static ref HTTP_CLIENT: reqwest::Client = reqwest::Client::builder().build().expect("failed to create http client");
}

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

/// Hmac type, expects 32 Byte hash
pub type HmacSha256 = Hmac<Sha256>;

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

pub fn check_sign<'a>(message: &'a [u8], secret: &[u8]) -> (bool, &'a [u8]) {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC can take key of any size!");
    mac.update(&message[32..]);
    let ok = mac.verify_slice(&message[..32]).is_ok();
    (ok, &message[32..])
}

pub fn generate_keep_alive() -> u64 {
    // retrieve a new mutable instance of an OS RNG
    let mut rng = OsRng;

    // generate random number
    rng.r#gen()
}

pub fn generate_keypair() -> Result<(RsaPrivateKey, RsaPublicKey), Error> {
    // retrieve a new mutable instance of an OS RNG
    let mut rng = OsRng;

    // generate the corresponding key pair
    let private_key = RsaPrivateKey::new(&mut rng, 1024)?;
    let public_key = RsaPublicKey::from(&private_key);

    // return the newly generated key pair
    Ok((private_key, public_key))
}

pub fn encode_public_key(key: &RsaPublicKey) -> Result<Vec<u8>, Error> {
    let encoded = key.to_public_key_der()?;

    Ok(encoded.to_vec())
}

pub fn decrypt(key: &RsaPrivateKey, value: &[u8]) -> Result<Vec<u8>, Error> {
    Ok(key.decrypt(Pkcs1v15Encrypt, value)?)
}

pub fn generate_token() -> Result<VerifyToken, Error> {
    // retrieve a new mutable instance of an OS RNG
    let mut rng = OsRng;

    // populate the random bytes
    let mut data = [0u8; 32];
    rng.try_fill_bytes(&mut data)?;

    Ok(data)
}

pub fn verify_token(expected: VerifyToken, actual: &[u8]) -> Result<(), Error> {
    if expected != actual {
        return Err(InvalidVerifyToken {
            expected,
            actual: actual.to_vec(),
        });
    }

    Ok(())
}

fn minecraft_hash(shared_secret: &[u8], encoded_public: &[u8]) -> String {
    // create a new hasher instance
    let mut hasher = Sha1::new();

    // server id
    hasher.update(b"");
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

pub async fn authenticate_mojang(
    username: &str,
    shared_secret: &[u8],
    encoded_public: &[u8],
) -> Result<AuthResponse, Error> {
    // calculate the minecraft hash for this secret, key and username
    let hash = minecraft_hash(shared_secret, encoded_public);

    // issue a request to Mojang's authentication endpoint
    let response = HTTP_CLIENT.get(format!("https://sessionserver.mojang.com/session/minecraft/hasJoined?username={username}&serverId={hash}"))
        .send()
        .await?
        .error_for_status()?;

    // extract the fields of the response
    Ok(response.json().await?)
}

pub fn create_ciphers(shared_secret: &[u8]) -> Result<(Aes128Cfb8Enc, Aes128Cfb8Dec), Error> {
    let encoder = Aes128Cfb8Enc::new_from_slices(shared_secret, shared_secret)?;
    let decoder = Aes128Cfb8Dec::new_from_slices(shared_secret, shared_secret)?;

    Ok((encoder, decoder))
}
