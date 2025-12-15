use crate::codec::Codec;
use bytes::{Buf, Bytes, BytesMut};
use compression_codecs::EncodeV2;
use compression_core::util::{PartialBuffer, WriteBuffer};
use http_body::{Body, Frame};
use pin_project_lite::pin_project;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

const OUTPUT_BUFFER_SIZE: usize = 8 * 1024; // 8KB output buffer

pin_project! {
    /// A response body that may be compressed.
    ///
    /// This type wraps an inner body and either compresses it using the
    /// specified codec or passes it through unchanged.
    #[project = CompressionBodyProj]
    #[allow(missing_docs)]
    pub enum CompressionBody<B> {
        /// Compressed body with encoder.
        Compressed {
            #[pin]
            inner: B,
            state: CompressedBody,
        },
        /// Passthrough body without compression.
        Passthrough {
            #[pin]
            inner: B,
        },
    }
}

/// State and buffers for an actively compressed body.
pub(crate) struct CompressedBody {
    encoder: Box<dyn EncodeV2 + Send>,
    output_buffer: Vec<u8>,
    always_flush: bool,
    state: CompressState,
    pending_trailers: Option<http::HeaderMap>,
}

/// State machine for compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompressState {
    /// Reading data from inner body and compressing.
    Reading,
    /// Finishing compression after inner body is done.
    Finishing,
    /// Emitting buffered trailers.
    Trailers,
    /// Compression is complete.
    Done,
}

impl CompressedBody {
    /// Creates a new compressed body state with the given codec.
    fn new(codec: Codec, always_flush: bool) -> Self {
        Self {
            encoder: codec.encoder(),
            output_buffer: vec![0u8; OUTPUT_BUFFER_SIZE],
            always_flush,
            state: CompressState::Reading,
            pending_trailers: None,
        }
    }

    /// Returns the current compression state.
    pub(crate) fn state(&self) -> CompressState {
        self.state
    }

    /// Returns whether always flush is enabled.
    #[allow(dead_code)]
    pub(crate) fn always_flush(&self) -> bool {
        self.always_flush
    }

    /// Polls the inner body and compresses data.
    fn poll_compressed<B>(
        &mut self,
        cx: &mut Context<'_>,
        mut inner: Pin<&mut B>,
    ) -> Poll<Option<Result<Frame<Bytes>, io::Error>>>
    where
        B: Body,
        B::Data: Buf,
        B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        loop {
            match self.state {
                CompressState::Done => return Poll::Ready(None),

                CompressState::Trailers => {
                    // Emit buffered trailers
                    if let Some(trailers) = self.pending_trailers.take() {
                        self.state = CompressState::Done;
                        return Poll::Ready(Some(Ok(Frame::trailers(trailers))));
                    } else {
                        self.state = CompressState::Done;
                        return Poll::Ready(None);
                    }
                }

                CompressState::Finishing => {
                    // Finish the encoder
                    let mut output =
                        WriteBuffer::new_initialized(self.output_buffer.as_mut_slice());

                    match self.encoder.finish(&mut output) {
                        Ok(done) => {
                            let written = output.written_len();
                            if written > 0 {
                                let data = Bytes::copy_from_slice(&self.output_buffer[..written]);
                                if done {
                                    self.state = if self.pending_trailers.is_some() {
                                        CompressState::Trailers
                                    } else {
                                        CompressState::Done
                                    };
                                }
                                return Poll::Ready(Some(Ok(Frame::data(data))));
                            } else if done {
                                self.state = if self.pending_trailers.is_some() {
                                    CompressState::Trailers
                                } else {
                                    CompressState::Done
                                };
                                continue;
                            }
                            // Continue looping to finish
                        }
                        Err(e) => {
                            return Poll::Ready(Some(Err(io::Error::other(e))));
                        }
                    }
                }

                CompressState::Reading => {
                    // Poll inner body for data
                    match inner.as_mut().poll_frame(cx) {
                        Poll::Pending => return Poll::Pending,
                        Poll::Ready(None) => {
                            // Inner body is done, transition to finishing
                            self.state = CompressState::Finishing;
                            continue;
                        }
                        Poll::Ready(Some(Err(e))) => {
                            return Poll::Ready(Some(Err(io::Error::other(e.into()))));
                        }
                        Poll::Ready(Some(Ok(frame))) => {
                            if let Some(data) = frame.data_ref() {
                                // Compress the data
                                let input_bytes = collect_bytes(data);
                                return self.compress_chunk(&input_bytes);
                            } else if let Ok(trailers) = frame.into_trailers() {
                                // Buffer trailers and finish compression first
                                self.pending_trailers = Some(trailers);
                                self.state = CompressState::Finishing;
                                continue;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Compresses a chunk of input data.
    fn compress_chunk(&mut self, input: &[u8]) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        let mut input_buf = PartialBuffer::new(input);
        let mut all_output = BytesMut::new();

        // Keep encoding until all input is consumed
        loop {
            let mut output = WriteBuffer::new_initialized(self.output_buffer.as_mut_slice());

            if let Err(e) = self.encoder.encode(&mut input_buf, &mut output) {
                return Poll::Ready(Some(Err(io::Error::other(e))));
            }

            let written = output.written_len();
            if written > 0 {
                all_output.extend_from_slice(&self.output_buffer[..written]);
            }

            // Check if we've consumed all input
            if input_buf.written_len() >= input.len() {
                break;
            }

            // Safety check to prevent infinite loop
            if written == 0 && input_buf.written_len() == 0 {
                break;
            }
        }

        // Flush if always_flush is enabled
        if self.always_flush {
            loop {
                let mut output = WriteBuffer::new_initialized(self.output_buffer.as_mut_slice());

                match self.encoder.flush(&mut output) {
                    Ok(done) => {
                        let written = output.written_len();
                        if written > 0 {
                            all_output.extend_from_slice(&self.output_buffer[..written]);
                        }
                        if done {
                            break;
                        }
                    }
                    Err(e) => {
                        return Poll::Ready(Some(Err(io::Error::other(e))));
                    }
                }
            }
        }

        if all_output.is_empty() {
            // No output yet, need to continue polling
            Poll::Pending
        } else {
            Poll::Ready(Some(Ok(Frame::data(all_output.freeze()))))
        }
    }
}

impl<B> CompressionBody<B> {
    /// Creates a compressed body with the given codec.
    pub fn compressed(inner: B, codec: Codec, always_flush: bool) -> Self {
        Self::Compressed {
            inner,
            state: CompressedBody::new(codec, always_flush),
        }
    }

    /// Creates a passthrough body without compression.
    pub fn passthrough(inner: B) -> Self {
        Self::Passthrough { inner }
    }
}

impl<B> Body for CompressionBody<B>
where
    B: Body,
    B::Data: Buf,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        match self.project() {
            CompressionBodyProj::Passthrough { inner } => {
                // Pass through frames, converting data to Bytes
                match inner.poll_frame(cx) {
                    Poll::Pending => Poll::Pending,
                    Poll::Ready(None) => Poll::Ready(None),
                    Poll::Ready(Some(Ok(frame))) => {
                        let frame = frame.map_data(|data| {
                            let mut bytes = BytesMut::with_capacity(data.remaining());
                            let mut chunk = data;
                            while chunk.has_remaining() {
                                let slice = chunk.chunk();
                                bytes.extend_from_slice(slice);
                                chunk.advance(slice.len());
                            }
                            bytes.freeze()
                        });
                        Poll::Ready(Some(Ok(frame)))
                    }
                    Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(io::Error::other(e.into())))),
                }
            }
            CompressionBodyProj::Compressed { inner, state } => state.poll_compressed(cx, inner),
        }
    }

    fn is_end_stream(&self) -> bool {
        match self {
            CompressionBody::Passthrough { inner } => inner.is_end_stream(),
            CompressionBody::Compressed { state, .. } => state.state() == CompressState::Done,
        }
    }

    fn size_hint(&self) -> http_body::SizeHint {
        match self {
            CompressionBody::Passthrough { inner } => inner.size_hint(),
            // Compressed size is unknown
            CompressionBody::Compressed { .. } => http_body::SizeHint::default(),
        }
    }
}

fn collect_bytes<D: Buf>(data: &D) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(data.remaining());
    let chunk = data.chunk();
    let remaining = data.remaining();
    let len = chunk.len().min(remaining);
    bytes.extend_from_slice(&chunk[..len]);
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderMap;
    use std::collections::VecDeque;

    /// A test body that yields predefined frames.
    struct TestBody {
        frames: VecDeque<Frame<Bytes>>,
    }

    impl TestBody {
        fn new(frames: Vec<Frame<Bytes>>) -> Self {
            Self {
                frames: frames.into(),
            }
        }
    }

    impl Body for TestBody {
        type Data = Bytes;
        type Error = std::convert::Infallible;

        fn poll_frame(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
            match self.frames.pop_front() {
                Some(frame) => Poll::Ready(Some(Ok(frame))),
                None => Poll::Ready(None),
            }
        }
    }

    fn poll_body<B: Body + Unpin>(body: &mut B) -> Option<Result<Frame<B::Data>, B::Error>> {
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        match Pin::new(body).poll_frame(&mut cx) {
            Poll::Ready(result) => result,
            Poll::Pending => None,
        }
    }

    #[test]
    fn test_passthrough_data() {
        let inner = TestBody::new(vec![Frame::data(Bytes::from("hello world"))]);
        let mut body = CompressionBody::passthrough(inner);

        let frame = poll_body(&mut body).unwrap().unwrap();
        assert!(frame.is_data());
        assert_eq!(frame.into_data().unwrap(), Bytes::from("hello world"));

        assert!(poll_body(&mut body).is_none());
    }

    #[test]
    fn test_passthrough_trailers() {
        let mut trailers = HeaderMap::new();
        trailers.insert("x-checksum", "abc123".parse().unwrap());

        let inner = TestBody::new(vec![
            Frame::data(Bytes::from("data")),
            Frame::trailers(trailers.clone()),
        ]);
        let mut body = CompressionBody::passthrough(inner);

        // First frame is data
        let frame = poll_body(&mut body).unwrap().unwrap();
        assert!(frame.is_data());

        // Second frame is trailers
        let frame = poll_body(&mut body).unwrap().unwrap();
        assert!(frame.is_trailers());
        let received_trailers = frame.into_trailers().unwrap();
        assert_eq!(received_trailers.get("x-checksum").unwrap(), "abc123");

        assert!(poll_body(&mut body).is_none());
    }

    #[test]
    #[cfg(feature = "gzip")]
    fn test_compressed_produces_output() {
        let inner = TestBody::new(vec![Frame::data(Bytes::from("hello world"))]);
        let mut body = CompressionBody::compressed(inner, Codec::Gzip, false);

        // Should get compressed data
        let frame = poll_body(&mut body).unwrap().unwrap();
        assert!(frame.is_data());
        let data = frame.into_data().unwrap();
        // Compressed output should exist (gzip header starts with 0x1f 0x8b)
        assert!(!data.is_empty());

        // Should get more data from finishing
        while let Some(Ok(frame)) = poll_body(&mut body) {
            assert!(frame.is_data());
        }
    }

    #[test]
    #[cfg(feature = "gzip")]
    fn test_compressed_with_trailers() {
        let mut trailers = HeaderMap::new();
        trailers.insert("x-checksum", "abc123".parse().unwrap());

        let inner = TestBody::new(vec![
            Frame::data(Bytes::from("hello world")),
            Frame::trailers(trailers),
        ]);
        let mut body = CompressionBody::compressed(inner, Codec::Gzip, false);

        // Collect all frames
        let mut data_frames = 0;
        let mut trailer_frame = None;
        while let Some(Ok(frame)) = poll_body(&mut body) {
            if frame.is_data() {
                data_frames += 1;
            } else if frame.is_trailers() {
                trailer_frame = Some(frame);
            }
        }

        // Should have received at least one data frame
        assert!(data_frames >= 1);

        // Should have received trailers
        let trailers = trailer_frame
            .expect("Expected trailers frame")
            .into_trailers()
            .unwrap();
        assert_eq!(trailers.get("x-checksum").unwrap(), "abc123");
    }
}
