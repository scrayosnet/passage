use aes::Aes128;
use aes::cipher::{Array, BlockModeDecrypt, BlockModeEncrypt, KeyIvInit};
use cfb8::{Decryptor, Encryptor};

/// A stream cipher applied to the raw byte stream.
///
/// The Minecraft protocol uses AES-128-CFB8, which is a byte-wise stream cipher with *separate*
/// encryption and decryption state.
pub trait Cipher: Send + 'static {
    /// Encrypts `buf` in place, advancing the encryption state.
    fn encrypt(&mut self, buf: &mut [u8]);

    /// Decrypts `buf` in place, advancing the decryption state.
    fn decrypt(&mut self, buf: &mut [u8]);
}

impl<C: Cipher + ?Sized> Cipher for Box<C> {
    fn encrypt(&mut self, buf: &mut [u8]) {
        (**self).encrypt(buf);
    }

    fn decrypt(&mut self, buf: &mut [u8]) {
        (**self).decrypt(buf);
    }
}

/// A cipher that does nothing.
pub struct NoCipher;

impl Cipher for NoCipher {
    fn encrypt(&mut self, _buf: &mut [u8]) {}
    fn decrypt(&mut self, _buf: &mut [u8]) {}
}

/// The length of a Minecraft shared secret, which is both the AES key and the CFB8 IV.
pub const SECRET_LEN: usize = 16;

/// The cipher the Minecraft protocol uses: AES-128-CFB8, keyed with the shared secret the client
/// sent in its encryption response and using that same secret as the IV.
pub struct Aes128Cfb8 {
    /// The encryption half, which is the only one that may touch outgoing bytes.
    encrypt: Encryptor<Aes128>,

    /// The decryption half. It has its own state, so the two directions must not be swapped.
    decrypt: Decryptor<Aes128>,
}

impl Aes128Cfb8 {
    /// Creates the cipher from a shared secret, or `None` unless it is [`SECRET_LEN`] bytes -- the
    /// one thing about it the peer controls, and so the caller's to blame it for.
    #[must_use]
    pub fn new(secret: &[u8]) -> Option<Self> {
        let secret: &[u8; SECRET_LEN] = secret.try_into().ok()?;
        Some(Self {
            encrypt: Encryptor::new(secret.into(), secret.into()),
            decrypt: Decryptor::new(secret.into(), secret.into()),
        })
    }
}

impl Cipher for Aes128Cfb8 {
    fn encrypt(&mut self, buf: &mut [u8]) {
        for byte in buf {
            self.encrypt.encrypt_block(Array::from_mut(byte));
        }
    }

    fn decrypt(&mut self, buf: &mut [u8]) {
        for byte in buf {
            self.decrypt.decrypt_block(Array::from_mut(byte));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stateful stand-in for AES-CFB8: the offset advances per byte, so a cipher called twice
    /// produces garbage rather than passing quietly.
    #[derive(Default)]
    struct Counting {
        encrypted: u8,
        decrypted: u8,
    }

    impl Cipher for Counting {
        fn encrypt(&mut self, buf: &mut [u8]) {
            for byte in buf {
                *byte = byte.wrapping_add(self.encrypted);
                self.encrypted = self.encrypted.wrapping_add(1);
            }
        }

        fn decrypt(&mut self, buf: &mut [u8]) {
            for byte in buf {
                *byte = byte.wrapping_sub(self.decrypted);
                self.decrypted = self.decrypted.wrapping_add(1);
            }
        }
    }

    #[test]
    fn the_two_directions_keep_their_own_state() {
        // CFB8 is a stream cipher with separate encryption and decryption state, so a single shared
        // counter would desynchronize a connection the moment both directions are used.
        let mut cipher = Counting::default();
        let mut buf = *b"abc";
        cipher.encrypt(&mut buf);
        assert_eq!(&buf, b"ace");

        let mut other = *b"abc";
        cipher.decrypt(&mut other);
        assert_eq!(&other, b"aaa", "decryption started from its own offset");
    }

    #[test]
    fn a_secret_the_client_made_up_is_refused_rather_than_panicking() {
        // Its length is the one thing the peer controls about the secret.
        assert!(Aes128Cfb8::new(&[0; SECRET_LEN]).is_some());
        assert!(Aes128Cfb8::new(b"short").is_none());
    }

    #[test]
    fn what_one_cipher_encrypts_the_other_decrypts() {
        // The two sides of a connection hold the same secret, and each one's encryption half is
        // read by the other's decryption half.
        let secret = b"0123456789abcdef";
        let mut server = Aes128Cfb8::new(secret).expect("a key");
        let mut client = Aes128Cfb8::new(secret).expect("a key");

        let mut first = *b"login success";
        server.encrypt(&mut first);
        assert_ne!(&first, b"login success", "it is on the wire encrypted");
        client.decrypt(&mut first);
        assert_eq!(&first, b"login success");

        // CFB8 is a stream cipher, so the second packet only decodes if both states advanced with
        // the first -- which is why the cipher lives for the whole connection.
        let mut second = *b"transfer";
        server.encrypt(&mut second);
        client.decrypt(&mut second);
        assert_eq!(&second, b"transfer");
    }

    #[test]
    fn a_boxed_cipher_is_the_same_cipher() {
        // The box implementation does not alter the internal cipher function.
        let mut plain = Counting::default();
        let mut boxed: Box<dyn Cipher> = Box::new(Counting::default());

        let mut expected = *b"hello";
        let mut actual = *b"hello";
        plain.encrypt(&mut expected);
        boxed.encrypt(&mut actual);
        assert_eq!(actual, expected);

        // And the state advanced on the boxed one too, rather than restarting.
        plain.encrypt(&mut expected);
        boxed.encrypt(&mut actual);
        assert_eq!(actual, expected);
    }
}
