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
