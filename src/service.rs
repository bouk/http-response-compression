use crate::codec::Codec;
use crate::future::ResponseFuture;
use http::Request;
use std::task::{Context, Poll};
use tower::Service;

/// A Tower service that compresses HTTP response bodies.
#[derive(Debug, Clone)]
pub struct CompressionService<S> {
    inner: S,
    min_size: usize,
}

impl<S> CompressionService<S> {
    /// Creates a new compression service wrapping the given inner service.
    pub fn new(inner: S, min_size: usize) -> Self {
        Self { inner, min_size }
    }

    /// Returns a reference to the inner service.
    pub fn inner(&self) -> &S {
        &self.inner
    }

    /// Returns a mutable reference to the inner service.
    pub fn inner_mut(&mut self) -> &mut S {
        &mut self.inner
    }

    /// Consumes this service, returning the inner service.
    pub fn into_inner(self) -> S {
        self.inner
    }
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for CompressionService<S>
where
    S: Service<Request<ReqBody>, Response = http::Response<ResBody>>,
{
    type Response = http::Response<crate::body::CompressionBody<ResBody>>;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        // Extract accepted codec from Accept-Encoding header
        let accepted_codec = req
            .headers()
            .get(http::header::ACCEPT_ENCODING)
            .and_then(|v| v.to_str().ok())
            .and_then(Codec::from_accept_encoding);

        let inner = self.inner.call(req);

        ResponseFuture::new(inner, accepted_codec, self.min_size)
    }
}
