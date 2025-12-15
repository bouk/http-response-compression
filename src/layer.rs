use crate::service::CompressionService;
use tower::Layer;

/// Default minimum body size for compression (approximately 1 MTU).
pub const DEFAULT_MIN_SIZE: usize = 860;

/// A Tower layer that compresses HTTP response bodies.
///
/// This layer wraps services and automatically compresses response bodies
/// based on the client's Accept-Encoding header.
#[derive(Debug, Clone)]
pub struct CompressionLayer {
    min_size: usize,
}

impl CompressionLayer {
    /// Creates a new compression layer with default settings.
    ///
    /// The default minimum size for compression is 860 bytes.
    pub fn new() -> Self {
        Self {
            min_size: DEFAULT_MIN_SIZE,
        }
    }

    /// Sets the minimum body size required for compression.
    ///
    /// Responses with a known Content-Length smaller than this value
    /// will not be compressed.
    pub fn min_size(mut self, size: usize) -> Self {
        self.min_size = size;
        self
    }
}

impl Default for CompressionLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> Layer<S> for CompressionLayer {
    type Service = CompressionService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        CompressionService::new(inner, self.min_size)
    }
}
