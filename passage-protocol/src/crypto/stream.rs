use crate::crypto::Error;
use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecryptMut, BlockEncryptMut};
use cfb8::cipher::BlockSizeUser;
use cfb8::cipher::KeyIvInit;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub type Aes128Cfb8Enc = cfb8::Encryptor<aes::Aes128>;
pub type Aes128Cfb8Dec = cfb8::Decryptor<aes::Aes128>;

/// Creates a cipher pair for encrypting a TCP stream. The pair is synced for the same shared secret.
pub fn create_ciphers(shared_secret: &[u8]) -> Result<(Aes128Cfb8Enc, Aes128Cfb8Dec), Error> {
    let encoder = Aes128Cfb8Enc::new_from_slices(shared_secret, shared_secret)?;
    let decoder = Aes128Cfb8Dec::new_from_slices(shared_secret, shared_secret)?;
    Ok((encoder, decoder))
}

/// A [`CipherStream`] is used to wrap a [`AsyncRead`] and [`AsyncWrite`] such that any bytes read
/// or written will be encrypted/decrypted using the provided block encryptor/decryptor.
pub struct CipherStream<S, E, D> {
    inner: S,
    encryptor: Option<E>,
    decryptor: Option<D>,
}

impl<S, E, D> CipherStream<S, E, D> {
    pub fn new(inner: S, encryptor: Option<E>, decryptor: Option<D>) -> Self {
        Self {
            inner,
            encryptor,
            decryptor,
        }
    }

    pub fn from_stream(inner: S) -> Self {
        Self::new(inner, None, None)
    }

    pub fn set_encryption(&mut self, encryptor: Option<E>, decryptor: Option<D>) {
        self.encryptor = encryptor;
        self.decryptor = decryptor;
    }

    pub fn is_encrypted(&self) -> bool {
        self.encryptor.is_some()
    }
}

impl<S> CipherStream<S, Aes128Cfb8Enc, Aes128Cfb8Dec> {
    pub fn from_secret(inner: S, shared_secret: &[u8]) -> Result<Self, Error> {
        let (encryptor, decryptor) = create_ciphers(shared_secret)?;
        Ok(Self::new(inner, Some(encryptor), Some(decryptor)))
    }
}

impl<S, E, D> AsyncWrite for CipherStream<S, E, D>
where
    S: AsyncWrite + Unpin,
    E: BlockEncryptMut + Unpin,
    D: BlockDecryptMut + Unpin,
{
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        let self_mut = self.get_mut();

        // if no encryptor present, use direct
        let Some(enc) = &mut self_mut.encryptor else {
            return Pin::new(&mut self_mut.inner).poll_write(cx, buf);
        };

        // encrypt buffer
        let mut buf = buf.to_vec();
        for chunk in buf.chunks_mut(Aes128Cfb8Enc::block_size()) {
            let gen_arr = GenericArray::from_mut_slice(chunk);
            enc.encrypt_block_mut(gen_arr);
        }

        // pass to inner
        Pin::new(&mut self_mut.inner).poll_write(cx, &buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), std::io::Error>> {
        // pass to inner
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        // pass to inner
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

impl<S, E, D> AsyncRead for CipherStream<S, E, D>
where
    S: AsyncRead + Unpin,
    E: BlockEncryptMut + Unpin,
    D: BlockDecryptMut + Unpin,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let self_mut = self.get_mut();

        let Some(dec) = &mut self_mut.decryptor else {
            return Pin::new(&mut self_mut.inner).poll_read(cx, buf);
        };

        // pass to inner
        let cursor = buf.capacity() - buf.remaining();
        let poll_result = Pin::new(&mut self_mut.inner).poll_read(cx, buf);

        // decrypt newly read buffer slice
        if poll_result.is_ready() {
            for chunk in buf.filled_mut()[cursor..].chunks_mut(Aes128Cfb8Dec::block_size()) {
                let gen_arr = GenericArray::from_mut_slice(chunk);
                dec.decrypt_block_mut(gen_arr);
            }
        }

        poll_result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::TryRngCore;
    use rand::rngs::SysRng;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const SHARED_SECRET: &[u8; 16] = b"verysecuresecret";

    fn generate_bytes(len: usize) -> Vec<u8> {
        let mut data = Vec::with_capacity(len);
        SysRng
            .try_fill_bytes(&mut data)
            .expect("failed to generate bytes");
        data
    }

    #[tokio::test]
    async fn without_encryption() {
        // create connected streams
        let (client_stream, server_stream) = tokio::io::duplex(1024);

        // wrap streams
        let mut client_stream: CipherStream<_, Aes128Cfb8Enc, Aes128Cfb8Dec> =
            CipherStream::new(client_stream, None, None);
        let mut server_stream: CipherStream<_, Aes128Cfb8Enc, Aes128Cfb8Dec> =
            CipherStream::new(server_stream, None, None);

        assert!(!client_stream.is_encrypted());
        assert!(!server_stream.is_encrypted());

        // send and receive packet
        let sent = generate_bytes(1024);
        client_stream
            .write_all(&sent)
            .await
            .expect("failed to send bytes");
        drop(client_stream);

        let mut received = Vec::with_capacity(1024);
        server_stream
            .read_to_end(&mut received)
            .await
            .expect("failed to receive bytes");

        assert_eq!(&sent, &received);
    }

    #[tokio::test]
    async fn with_encryption() {
        // create connected streams
        let (client_stream, server_stream) = tokio::io::duplex(1024);

        // wrap streams
        let (encryptor, decryptor) =
            create_ciphers(SHARED_SECRET).expect("failed to create ciphers");
        let mut client_stream = CipherStream::new(client_stream, Some(encryptor), Some(decryptor));
        let (encryptor, decryptor) =
            create_ciphers(SHARED_SECRET).expect("failed to create ciphers");
        let mut server_stream = CipherStream::new(server_stream, Some(encryptor), Some(decryptor));

        assert!(client_stream.is_encrypted());
        assert!(server_stream.is_encrypted());

        // send and receive packet
        let sent = generate_bytes(1024);
        client_stream
            .write_all(&sent)
            .await
            .expect("failed to send bytes");
        drop(client_stream);

        let mut received = Vec::with_capacity(1024);
        server_stream
            .read_to_end(&mut received)
            .await
            .expect("failed to receive bytes");

        assert_eq!(&sent, &received);
    }

    #[tokio::test]
    async fn with_some_encryption() {
        // create connected streams
        let (client_stream, server_stream) = tokio::io::duplex(1024);

        // wrap streams
        let mut client_stream = CipherStream::new(client_stream, None, None);
        let mut server_stream = CipherStream::new(server_stream, None, None);

        assert!(!client_stream.is_encrypted());
        assert!(!server_stream.is_encrypted());

        // send and receive packet
        let sent = generate_bytes(1024);
        client_stream
            .write_all(&sent)
            .await
            .expect("failed to send bytes");

        let mut received = Vec::with_capacity(1024);
        server_stream
            .read_exact(&mut received)
            .await
            .expect("failed to receive bytes");

        // enable encryption
        let (encryptor, decryptor) =
            create_ciphers(SHARED_SECRET).expect("failed to create ciphers");
        client_stream.set_encryption(Some(encryptor), Some(decryptor));
        let (encryptor, decryptor) =
            create_ciphers(SHARED_SECRET).expect("failed to create ciphers");
        server_stream.set_encryption(Some(encryptor), Some(decryptor));

        assert!(client_stream.is_encrypted());
        assert!(server_stream.is_encrypted());

        // send and receive packet
        let sent = generate_bytes(1024);
        client_stream
            .write_all(&sent)
            .await
            .expect("failed to send bytes");
        drop(client_stream);

        let mut received = Vec::with_capacity(1024);
        server_stream
            .read_to_end(&mut received)
            .await
            .expect("failed to receive bytes");

        assert_eq!(&sent, &received);
    }
}
