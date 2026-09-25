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

// TODO implement a proper cipher.

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
