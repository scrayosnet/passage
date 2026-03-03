use crate::{AsyncReadPacket, AsyncWritePacket, Error, ReadPacket, VarInt};
use bytes::{Buf, BytesMut};
use std::io::Cursor;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::AsyncWrite;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

/// The initial buffer size for the packet stream internal buffers.
static INITIAL_BUFFER_SIZE: usize = 1_024;

/// A wrapper implementing [`AsyncRead`] for a [`Vec<u8>`]. The internal buffer automatically grows.
pub(crate) struct VecAsyncWriter {
    buf: Vec<u8>,
}

impl VecAsyncWriter {
    /// Creates a new writer with an initial capacity.
    pub(crate) fn with_capacity(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap),
        }
    }

    /// Clears the buffer, removing all its contents.
    pub(crate) fn clear(&mut self) {
        self.buf.clear();
    }

    /// Returns the length of the buffer.
    pub(crate) fn len(&self) -> usize {
        self.buf.len()
    }

    /// Returns the underlying buffer as a slice.
    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.buf
    }
}

impl AsyncWrite for VecAsyncWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.buf.extend_from_slice(data);
        Poll::Ready(Ok(data.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

pub struct PacketStream<S> {
    stream: S,
    max_packet_size: usize,
    write_buffer: VecAsyncWriter,
    read_buffer: BytesMut,
}

/// [`PacketStream`] wraps an async stream, providing methods to read and write packets. Most notably,
/// the [`PacketStream::read_packet`] method is **cancellation safe**.
impl<S> PacketStream<S> {
    pub fn new(stream: S, max_packet_size: usize) -> Self {
        Self {
            stream,
            max_packet_size,
            write_buffer: VecAsyncWriter::with_capacity(INITIAL_BUFFER_SIZE),
            read_buffer: BytesMut::with_capacity(INITIAL_BUFFER_SIZE),
        }
    }

    /// Returns whether the internal buffer is currently empty. This should only be false after cancelling
    /// a packet read without completing it afterward.
    pub fn is_empty(&self) -> bool {
        self.read_buffer.is_empty()
    }

    /// Returns the internal stream.
    pub fn inner(&self) -> &S {
        &self.stream
    }

    /// Returns the internal stream (mutable).
    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.stream
    }
}

impl<S: AsyncRead + Unpin + Send + Sync> PacketStream<S> {
    /// Reads exactly all bytes of a packet into memory. The resulting buffer starts with the packet ID.
    /// On error, the stream becomes unusable. The method returns the read buffer or `None` if the
    /// stream was closed (by the peer) and no packet was in transit. When the connection was closed
    /// and a packet was in transit, then the method returns an [`Error::ConnectionClosed`] error.
    ///
    /// This method is **cancellation safe** and may be used in, for example, a [`tokio::select`].
    pub async fn read_packet(&mut self) -> Result<Option<Vec<u8>>, Error> {
        // Continuously read bytes until the length is filled. The all read bytes are stored in an
        // internal buffer. This makes the method cancellation safe.
        // This implementation is based on mini-redis:
        // https://github.com/tokio-rs/mini-redis/blob/e186482ca00f8d884ddcbe20417f3654d03315a4/src/connection.rs#L56
        loop {
            println!("buffer length {:?}", self.read_buffer.len());

            // Try to parse the packet length from the buffer.
            let mut reader = Cursor::new(&self.read_buffer);
            let length = reader
                .read_varint()
                .await
                .ok()
                // Add the length of the length field to the full length.
                .map(|n| n as usize + reader.position() as usize);
            println!("packet length {:?}", length);

            // Check whether the full packet has been read
            if let Some(length) = length
                && self.read_buffer.len() == length
            {
                println!("packet completed {:?}", length);
                // Advance the buffer to the end of the 'length' VarInt as it should not be included.
                self.read_buffer.advance(reader.position() as usize);
                let packet = self.read_buffer.to_vec();
                self.read_buffer.clear();
                return Ok(Some(packet));
            }

            // Stop if the packet size is too large. It does not reset the buffer, making the stream
            // unusable. Any following call to the stream will result in the same error.
            if self.read_buffer.len() > self.max_packet_size {
                println!("packet too large {:?}", length);
                return Err(Error::IllegalPacketLength);
            }

            // Otherwise try to read the remaining bytes of the packet or only on byte if the
            // length is not yet known.
            let read_up_to = length.unwrap_or_else(|| self.read_buffer.len() + 1);
            let read = (&mut self.stream)
                .take((read_up_to - self.read_buffer.len()) as u64)
                .read_buf(&mut self.read_buffer)
                .await?;
            println!("read bytes {:?}", read);

            if read == 0 {
                // If no bytes were read, then the stream is closed. This should only happen after
                // every packet has been read in full.
                if !self.read_buffer.is_empty() {
                    return Err(Error::ConnectionClosed);
                }
                return Ok(None);
            }
        }
    }

    /// Reads exactly all bytes of a packet into memory and parses it. On error, the stream becomes
    /// unusable.
    ///
    /// This method is **partially cancellation safe** as it ensures that the stream is not compromised,
    /// but it might drop the read packet while parsing.
    pub async fn parse_packet<T: ReadPacket + Send + Sync>(&mut self) -> Result<T, Error> {
        // read the packet buffer
        let buffer = self.read_packet().await?.ok_or(Error::ConnectionClosed)?;
        let mut reader = Cursor::new(&buffer);

        // read the packet
        let packet_id = reader.read_varint().await?;
        if packet_id != T::ID {
            return Err(Error::IllegalPacketId {
                expected: vec![T::ID],
                actual: packet_id,
            });
        }
        T::read_from_buffer(&mut reader).await
    }
}

impl<S: AsyncWrite + Unpin + Send + Sync> PacketStream<S> {
    /// Writes a packet to the stream. This method os **not cancellation safe**.
    pub async fn write_packet<T: crate::WritePacket + Send + Sync + 'static>(
        &mut self,
        packet: T,
    ) -> Result<usize, Error> {
        // Reset the workhorse buffer
        self.write_buffer.clear();

        // Parse the packet into a buffer
        self.write_buffer.write_varint(T::ID as VarInt).await?;
        packet.write_to_buffer(&mut self.write_buffer).await?;

        // Write the packet length separately
        let packet_len = self.write_buffer.len();
        self.stream.write_varint(packet_len as VarInt).await?;

        // Write the rest of the packet
        self.stream.write_all(self.write_buffer.as_slice()).await?;
        Ok(self.write_buffer.len())
    }
}
