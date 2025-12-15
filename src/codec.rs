use compression_codecs::{
    EncodeV2,
    brotli::{BrotliEncoder, params::EncoderParams as BrotliParams},
    deflate::DeflateEncoder,
    gzip::GzipEncoder,
    zstd::ZstdEncoder,
};
use compression_core::Level;

/// Supported compression codecs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// Zstd compression.
    Zstd,
    /// Brotli compression.
    Brotli,
    /// Gzip compression.
    Gzip,
    /// Deflate compression.
    Deflate,
}

impl Codec {
    /// Returns the Content-Encoding header value for this codec.
    pub fn content_encoding(&self) -> &'static str {
        match self {
            Codec::Zstd => "zstd",
            Codec::Brotli => "br",
            Codec::Gzip => "gzip",
            Codec::Deflate => "deflate",
        }
    }

    /// Creates a new encoder for this codec.
    pub fn encoder(&self) -> Box<dyn EncodeV2 + Send> {
        match self {
            Codec::Zstd => Box::new(ZstdEncoder::new(3)), // level 3 is a good default
            Codec::Brotli => Box::new(BrotliEncoder::new(BrotliParams::default())),
            Codec::Gzip => Box::new(GzipEncoder::new(Level::Default.into())),
            Codec::Deflate => Box::new(DeflateEncoder::new(Level::Default.into())),
        }
    }

    /// Parses the Accept-Encoding header and returns the best supported codec.
    ///
    /// The header value is expected to be comma-separated encodings with optional
    /// quality values (e.g., "gzip, br;q=1.0, zstd;q=0.8").
    pub fn from_accept_encoding(header: &str) -> Option<Codec> {
        let mut best_codec: Option<(Codec, f32)> = None;

        for part in header.split(',') {
            let part = part.trim();
            let (encoding, quality) = parse_encoding_with_quality(part);

            // Skip if quality is 0
            if quality == 0.0 {
                continue;
            }

            let codec = match encoding {
                "zstd" => Some(Codec::Zstd),
                "br" | "brotli" => Some(Codec::Brotli),
                "gzip" | "x-gzip" => Some(Codec::Gzip),
                "deflate" => Some(Codec::Deflate),
                _ => None,
            };

            if let Some(codec) = codec {
                match &best_codec {
                    None => best_codec = Some((codec, quality)),
                    Some((_, best_quality)) if quality > *best_quality => {
                        best_codec = Some((codec, quality));
                    }
                    Some((_, best_quality)) if quality == *best_quality => {
                        // Prefer zstd > brotli > gzip > deflate when quality is equal
                        let priority = |c: &Codec| match c {
                            Codec::Zstd => 0,
                            Codec::Brotli => 1,
                            Codec::Gzip => 2,
                            Codec::Deflate => 3,
                        };
                        if priority(&codec) < priority(&best_codec.as_ref().unwrap().0) {
                            best_codec = Some((codec, quality));
                        }
                    }
                    _ => {}
                }
            }
        }

        best_codec.map(|(codec, _)| codec)
    }
}

/// Parses an encoding entry like "gzip" or "br;q=0.8" into (encoding, quality).
fn parse_encoding_with_quality(s: &str) -> (&str, f32) {
    let mut parts = s.splitn(2, ';');
    let encoding = parts.next().unwrap_or("").trim();

    let quality = parts
        .next()
        .and_then(|q| {
            let q = q.trim();
            if q.starts_with("q=") || q.starts_with("Q=") {
                q[2..].parse::<f32>().ok()
            } else {
                None
            }
        })
        .unwrap_or(1.0);

    (encoding, quality)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_encoding() {
        assert_eq!(Codec::Zstd.content_encoding(), "zstd");
        assert_eq!(Codec::Brotli.content_encoding(), "br");
        assert_eq!(Codec::Gzip.content_encoding(), "gzip");
        assert_eq!(Codec::Deflate.content_encoding(), "deflate");
    }

    #[test]
    fn test_from_accept_encoding_simple() {
        assert_eq!(Codec::from_accept_encoding("zstd"), Some(Codec::Zstd));
        assert_eq!(Codec::from_accept_encoding("br"), Some(Codec::Brotli));
        assert_eq!(Codec::from_accept_encoding("gzip"), Some(Codec::Gzip));
        assert_eq!(Codec::from_accept_encoding("deflate"), Some(Codec::Deflate));
    }

    #[test]
    fn test_from_accept_encoding_multiple() {
        // With equal quality, prefer zstd
        assert_eq!(
            Codec::from_accept_encoding("gzip, br, zstd"),
            Some(Codec::Zstd)
        );
    }

    #[test]
    fn test_from_accept_encoding_with_quality() {
        assert_eq!(
            Codec::from_accept_encoding("gzip;q=1.0, br;q=0.5"),
            Some(Codec::Gzip)
        );
        assert_eq!(
            Codec::from_accept_encoding("gzip;q=0.5, br;q=1.0"),
            Some(Codec::Brotli)
        );
    }

    #[test]
    fn test_from_accept_encoding_unsupported() {
        assert_eq!(Codec::from_accept_encoding("identity"), None);
        assert_eq!(Codec::from_accept_encoding("compress"), None);
    }

    #[test]
    fn test_from_accept_encoding_quality_zero() {
        assert_eq!(Codec::from_accept_encoding("gzip;q=0"), None);
        assert_eq!(
            Codec::from_accept_encoding("gzip;q=0, br"),
            Some(Codec::Brotli)
        );
    }
}
