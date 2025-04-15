use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecryptMut, BlockEncryptMut};
use cfb8::cipher::BlockSizeUser;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub type Aes128Cfb8Enc = cfb8::Encryptor<aes::Aes128>;
pub type Aes128Cfb8Dec = cfb8::Decryptor<aes::Aes128>;

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

    pub fn set_encryption(&mut self, encryptor: Option<E>, decryptor: Option<D>) {
        self.encryptor = encryptor;
        self.decryptor = decryptor;
    }

    pub fn is_encrypted(&self) -> bool {
        self.encryptor.is_some()
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
