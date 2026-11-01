//! JSONL and LSP `Content-Length` framing on one stream.

use std::io::{self, BufRead, Write};

/// Hard cap on one JSON-RPC body.
pub const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
/// Hard cap on LSP header lines.
pub const MAX_HEADER_LINES: usize = 32;

/// How the peer framed the last message. Replies use the same style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framing {
    /// One JSON body plus a single trailing newline.
    Jsonl,
    /// LSP-style `Content-Length` headers, then the body bytes.
    Headers,
}

/// A framing read that cannot produce a body.
#[derive(Debug)]
pub enum FrameError {
    /// Underlying reader or writer failed.
    Io(io::Error),
    /// Header block exceeded [`MAX_HEADER_LINES`].
    TooManyHeaders,
    /// Header block ended without a `Content-Length`.
    MissingContentLength,
    /// `Content-Length` is not a decimal usize.
    InvalidContentLength,
    /// Body is larger than [`MAX_MESSAGE_BYTES`].
    MessageTooLarge,
    /// Stream ended before a complete frame.
    Incomplete,
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::Io(err) => write!(f, "{err}"),
            FrameError::TooManyHeaders => write!(f, "too many framing headers"),
            FrameError::MissingContentLength => write!(f, "missing Content-Length"),
            FrameError::InvalidContentLength => write!(f, "invalid Content-Length"),
            FrameError::MessageTooLarge => write!(f, "message exceeds size limit"),
            FrameError::Incomplete => write!(f, "incomplete frame"),
        }
    }
}

impl std::error::Error for FrameError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FrameError::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<io::Error> for FrameError {
    fn from(err: io::Error) -> Self {
        if err.kind() == io::ErrorKind::UnexpectedEof {
            FrameError::Incomplete
        } else {
            FrameError::Io(err)
        }
    }
}

/// First-line rule: `^[A-Za-z][A-Za-z0-9-]*:` is headers.
/// Anything else, including a non-`{` JSONL body, is JSONL.
pub fn is_header_line(line: &[u8]) -> bool {
    let line = trim_crlf(line);
    let mut bytes = line.iter();
    match bytes.next() {
        Some(b) if b.is_ascii_alphabetic() => {}
        _ => return false,
    }
    for b in bytes {
        if *b == b':' {
            return true;
        }
        if !b.is_ascii_alphanumeric() && *b != b'-' {
            return false;
        }
    }
    false
}

/// Classify the first line the way [`read_message`] does.
pub fn classify_first_line(line: &[u8]) -> Framing {
    if is_header_line(line) {
        Framing::Headers
    } else {
        Framing::Jsonl
    }
}

/// Write `payload` using `framing`. Does not append a second newline to JSON.
///
/// # Errors
///
/// Returns an error when `payload` exceeds [`MAX_MESSAGE_BYTES`] or the writer
/// fails.
pub fn write_message<W: Write>(writer: &mut W, payload: &[u8], framing: Framing) -> io::Result<()> {
    if payload.len() > MAX_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "message exceeds size limit",
        ));
    }
    match framing {
        Framing::Jsonl => {
            writer.write_all(payload)?;
            writer.write_all(b"\n")?;
        }
        Framing::Headers => {
            let header = format!("Content-Length: {}\r\n\r\n", payload.len());
            writer.write_all(header.as_bytes())?;
            writer.write_all(payload)?;
        }
    }
    Ok(())
}

/// Read one framed body. The returned [`Framing`] is what the reply must use.
///
/// The cap is the body size: a JSONL payload of [`MAX_MESSAGE_BYTES`] plus its
/// terminating newline (or CRLF) is accepted. That matches [`write_message`].
///
/// # Errors
///
/// Returns an error when the stream ends mid-frame, the header block is too
/// long or lacks a valid `Content-Length`, the body exceeds
/// [`MAX_MESSAGE_BYTES`], or the reader fails.
pub fn read_message<R: BufRead>(reader: &mut R) -> Result<(Vec<u8>, Framing), FrameError> {
    let first = read_line_limited(reader, MAX_MESSAGE_BYTES + 2)?;
    if is_header_line(&first) {
        let length = read_content_length(reader, first)?;
        let mut body = vec![0u8; length];
        reader.read_exact(&mut body)?;
        Ok((body, Framing::Headers))
    } else {
        let body = strip_crlf(first);
        if body.len() > MAX_MESSAGE_BYTES {
            return Err(FrameError::MessageTooLarge);
        }
        Ok((body, Framing::Jsonl))
    }
}

fn read_content_length<R: BufRead>(reader: &mut R, first: Vec<u8>) -> Result<usize, FrameError> {
    let mut header = first;
    let mut count = 0;
    let mut length = None;
    loop {
        if is_blank_line(&header) {
            break;
        }
        count += 1;
        if count > MAX_HEADER_LINES {
            return Err(FrameError::TooManyHeaders);
        }
        if let Some(parsed) = parse_content_length_line(&header) {
            length = Some(parsed?);
        }
        header = read_line_limited(reader, MAX_MESSAGE_BYTES + 1)?;
    }
    match length {
        None => Err(FrameError::MissingContentLength),
        Some(n) if n > MAX_MESSAGE_BYTES => Err(FrameError::MessageTooLarge),
        Some(n) => Ok(n),
    }
}

fn parse_content_length_line(line: &[u8]) -> Option<Result<usize, FrameError>> {
    let line = trim_crlf(line);
    let colon = line.iter().position(|&b| b == b':')?;
    let name = trim_ascii(&line[..colon]);
    if !name.eq_ignore_ascii_case(b"content-length") {
        return None;
    }
    let value = trim_ascii(&line[colon + 1..]);
    let text = match std::str::from_utf8(value) {
        Ok(s) => s,
        Err(_) => return Some(Err(FrameError::InvalidContentLength)),
    };
    match text.parse::<usize>() {
        Ok(n) => Some(Ok(n)),
        Err(_) => Some(Err(FrameError::InvalidContentLength)),
    }
}

fn read_line_limited<R: BufRead>(reader: &mut R, max: usize) -> Result<Vec<u8>, FrameError> {
    let mut buf = Vec::new();
    loop {
        let avail = reader.fill_buf()?;
        if avail.is_empty() {
            if buf.is_empty() {
                return Err(FrameError::Incomplete);
            }
            return Ok(buf);
        }
        if let Some(pos) = avail.iter().position(|&b| b == b'\n') {
            let take = pos + 1;
            if buf.len() + take > max {
                return Err(FrameError::MessageTooLarge);
            }
            buf.extend_from_slice(&avail[..take]);
            reader.consume(take);
            return Ok(buf);
        }
        if buf.len() + avail.len() > max {
            return Err(FrameError::MessageTooLarge);
        }
        buf.extend_from_slice(avail);
        let n = avail.len();
        reader.consume(n);
    }
}

fn is_blank_line(line: &[u8]) -> bool {
    matches!(trim_crlf(line), b"")
}

fn strip_crlf(mut line: Vec<u8>) -> Vec<u8> {
    if line.last() == Some(&b'\n') {
        line.pop();
        if line.last() == Some(&b'\r') {
            line.pop();
        }
    }
    line
}

fn trim_crlf(line: &[u8]) -> &[u8] {
    let mut end = line.len();
    if end > 0 && line[end - 1] == b'\n' {
        end -= 1;
        if end > 0 && line[end - 1] == b'\r' {
            end -= 1;
        }
    }
    &line[..end]
}

fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map(|i| i + 1)
        .unwrap_or(0);
    if start >= end {
        &[]
    } else {
        &bytes[start..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn encode_decode(payload: &[u8], framing: Framing) -> (Vec<u8>, Framing) {
        let mut buf = Vec::new();
        write_message(&mut buf, payload, framing).unwrap();
        let mut cur = Cursor::new(buf);
        read_message(&mut cur).unwrap()
    }

    #[test]
    fn jsonl_request_roundtrips() {
        let payload = br#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#;
        let (got, framing) = encode_decode(payload, Framing::Jsonl);
        assert_eq!(framing, Framing::Jsonl);
        assert_eq!(got, payload);
    }

    #[test]
    fn content_length_request_roundtrips() {
        let payload = br#"{"jsonrpc":"2.0","id":1,"method":"identity/get"}"#;
        let (got, framing) = encode_decode(payload, Framing::Headers);
        assert_eq!(framing, Framing::Headers);
        assert_eq!(got, payload);
    }

    #[test]
    fn first_line_that_is_not_brace_is_still_jsonl() {
        let payload = br#""not-an-object""#;
        assert_eq!(classify_first_line(payload), Framing::Jsonl);
        assert_eq!(classify_first_line(b"[1,2,3]\n"), Framing::Jsonl);
        assert_eq!(classify_first_line(b"\n"), Framing::Jsonl);
        assert_eq!(
            classify_first_line(b"\xef\xbb\xbf{\"a\":1}"),
            Framing::Jsonl
        );
        let (got, framing) = encode_decode(payload, Framing::Jsonl);
        assert_eq!(framing, Framing::Jsonl);
        assert_eq!(got, payload);
    }

    #[test]
    fn header_line_is_classified_as_headers() {
        assert_eq!(
            classify_first_line(b"Content-Length: 12\r\n"),
            Framing::Headers
        );
        assert_eq!(
            classify_first_line(b"Content-Type: application/vscode-jsonrpc; charset=utf-8\r\n"),
            Framing::Headers
        );
        assert!(is_header_line(b"Content-Length: 1"));
        assert!(!is_header_line(b"{"));
        assert!(!is_header_line(b"1: not a header name start"));
    }

    #[test]
    fn headers_accept_content_type_before_length() {
        let payload = br#"{"ok":true}"#;
        let mut buf = Vec::new();
        buf.extend_from_slice(b"Content-Type: application/vscode-jsonrpc\r\n");
        buf.extend_from_slice(format!("Content-Length: {}\r\n\r\n", payload.len()).as_bytes());
        buf.extend_from_slice(payload);
        let (got, framing) = read_message(&mut Cursor::new(buf)).unwrap();
        assert_eq!(framing, Framing::Headers);
        assert_eq!(got, payload);
    }

    #[test]
    fn missing_content_length_is_an_error() {
        let buf = b"Content-Type: application/json\r\n\r\n";
        let err = read_message(&mut Cursor::new(&buf[..])).unwrap_err();
        assert!(matches!(err, FrameError::MissingContentLength));
    }

    #[test]
    fn invalid_content_length_is_an_error() {
        let buf = b"Content-Length: nope\r\n\r\n";
        let err = read_message(&mut Cursor::new(&buf[..])).unwrap_err();
        assert!(matches!(err, FrameError::InvalidContentLength));
    }

    #[test]
    fn too_many_headers_is_an_error() {
        let mut buf = Vec::new();
        for _ in 0..(MAX_HEADER_LINES + 1) {
            buf.extend_from_slice(b"X-Extra: 1\r\n");
        }
        buf.extend_from_slice(b"\r\n");
        let err = read_message(&mut Cursor::new(buf)).unwrap_err();
        assert!(matches!(err, FrameError::TooManyHeaders));
    }

    #[test]
    fn oversized_jsonl_is_rejected() {
        let mut line = vec![b'x'; MAX_MESSAGE_BYTES + 2];
        line.push(b'\n');
        let err = read_message(&mut Cursor::new(line)).unwrap_err();
        assert!(matches!(err, FrameError::MessageTooLarge));
    }

    #[test]
    fn oversized_content_length_is_rejected() {
        let buf = format!("Content-Length: {}\r\n\r\n", MAX_MESSAGE_BYTES + 1);
        let err = read_message(&mut Cursor::new(buf.into_bytes())).unwrap_err();
        assert!(matches!(err, FrameError::MessageTooLarge));
    }

    #[test]
    fn empty_reader_is_incomplete() {
        let err = read_message(&mut Cursor::new(&b""[..])).unwrap_err();
        assert!(matches!(err, FrameError::Incomplete));
    }

    #[test]
    fn exact_max_payload_roundtrips_jsonl_and_headers() {
        let payload = vec![b'a'; MAX_MESSAGE_BYTES];
        let (got, framing) = encode_decode(&payload, Framing::Jsonl);
        assert_eq!(framing, Framing::Jsonl);
        assert_eq!(got, payload);
        let (got, framing) = encode_decode(&payload, Framing::Headers);
        assert_eq!(framing, Framing::Headers);
        assert_eq!(got, payload);
    }

    #[test]
    fn write_rejects_oversized_payload() {
        let payload = vec![b'a'; MAX_MESSAGE_BYTES + 1];
        let err = write_message(&mut Vec::new(), &payload, Framing::Jsonl).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn frame_error_display_names_the_limit() {
        assert_eq!(
            FrameError::MessageTooLarge.to_string(),
            "message exceeds size limit"
        );
        assert_eq!(
            FrameError::TooManyHeaders.to_string(),
            "too many framing headers"
        );
    }
}
