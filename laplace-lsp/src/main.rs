mod analysis;
mod backend;
mod builtins;
mod completion;
mod diagnostics;
mod position;
mod stanc;
mod workspace;

use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, ReadBuf};
use tower_lsp::{LspService, Server};

/// Wraps an `AsyncRead` and discards any leading ASCII-whitespace bytes (e.g. a
/// bare newline sent by a terminal) before the first real byte arrives.
///
/// This works around a footgun in the `tower-lsp` + `tokio-util` combo: once
/// `LanguageServerCodec::decode` returns any `Err` (e.g. because the very first
/// bytes on stdin aren't a `Content-Length` header), `tokio_util`'s `FramedRead`
/// permanently kills the stream on the *next* poll (see
/// https://github.com/tokio-rs/tokio/issues/3976), even though the codec itself
/// is designed to skip garbage and keep going. In practice this means one stray
/// byte before the client starts speaking LSP - e.g. pressing Enter while
/// testing the server directly in a terminal - makes it reply with a single
/// `-32700 Parse error` and exit, instead of waiting for real input.
struct SkipLeadingWhitespace<R> {
    inner: R,
    skipping: bool,
}

impl<R: AsyncRead + Unpin> AsyncRead for SkipLeadingWhitespace<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if !self.skipping {
            return Pin::new(&mut self.inner).poll_read(cx, buf);
        }

        loop {
            let mut byte = [0u8; 1];
            let mut scratch = ReadBuf::new(&mut byte);
            match Pin::new(&mut self.inner).poll_read(cx, &mut scratch) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(err)) => return Poll::Ready(Err(err)),
                Poll::Ready(Ok(())) => {}
            }

            match scratch.filled().first().copied() {
                None => {
                    // EOF before any real input arrived.
                    self.skipping = false;
                    return Poll::Ready(Ok(()));
                }
                Some(b) if b.is_ascii_whitespace() => continue,
                Some(b) => {
                    self.skipping = false;
                    buf.put_slice(&[b]);
                    return Poll::Ready(Ok(()));
                }
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let stdin = SkipLeadingWhitespace {
        inner: tokio::io::stdin(),
        skipping: true,
    };
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(backend::Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn skips_leading_whitespace() {
        let mut reader = SkipLeadingWhitespace {
            inner: std::io::Cursor::new(b"\r\n\r\nContent-Length: 2\r\n\r\n{}".to_vec()),
            skipping: true,
        };
        let mut out = Vec::new();
        reader.read_to_end(&mut out).await.unwrap();
        assert_eq!(out, b"Content-Length: 2\r\n\r\n{}");
    }

    #[tokio::test]
    async fn passes_through_when_no_leading_whitespace() {
        let mut reader = SkipLeadingWhitespace {
            inner: std::io::Cursor::new(b"Content-Length: 2\r\n\r\n{}".to_vec()),
            skipping: true,
        };
        let mut out = Vec::new();
        reader.read_to_end(&mut out).await.unwrap();
        assert_eq!(out, b"Content-Length: 2\r\n\r\n{}");
    }

    #[tokio::test]
    async fn handles_eof_with_only_whitespace() {
        let mut reader = SkipLeadingWhitespace {
            inner: std::io::Cursor::new(b"\r\n\r\n".to_vec()),
            skipping: true,
        };
        let mut out = Vec::new();
        reader.read_to_end(&mut out).await.unwrap();
        assert_eq!(out, b"");
    }
}
