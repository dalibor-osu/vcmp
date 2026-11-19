use std::io::SeekFrom;
use std::path::Path;
use std::time::UNIX_EPOCH;

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::io::ReaderStream;
use vcmp_library::Format;
use vcmp_protocol::{ErrorCode, audio_etag};

use crate::api::Failure;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ByteRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RangeRequest {
    Whole,
    Part(ByteRange),
    Unsatisfiable,
}

pub(crate) fn parse_range(header: Option<&str>, size: u64) -> RangeRequest {
    let Some(spec) = header.and_then(|value| value.trim().strip_prefix("bytes=")) else {
        return RangeRequest::Whole;
    };
    if spec.contains(',') {
        return RangeRequest::Whole;
    }
    let Some((first, last)) = spec.split_once('-') else {
        return RangeRequest::Whole;
    };
    let (first, last) = (first.trim(), last.trim());
    let range = if first.is_empty() {
        let Ok(suffix) = last.parse::<u64>() else {
            return RangeRequest::Whole;
        };
        if suffix == 0 || size == 0 {
            return RangeRequest::Unsatisfiable;
        }
        ByteRange {
            start: size.saturating_sub(suffix),
            end: size - 1,
        }
    } else {
        let Ok(start) = first.parse::<u64>() else {
            return RangeRequest::Whole;
        };
        let end = if last.is_empty() {
            size.saturating_sub(1)
        } else {
            match last.parse::<u64>() {
                Ok(end) if end >= start => end.min(size.saturating_sub(1)),
                _ => return RangeRequest::Whole,
            }
        };
        if start >= size {
            return RangeRequest::Unsatisfiable;
        }
        ByteRange { start, end }
    };
    RangeRequest::Part(range)
}

pub(crate) fn content_type(format: Option<Format>) -> &'static str {
    match format {
        Some(Format::Mp3) => "audio/mpeg",
        Some(Format::Flac) => "audio/flac",
        Some(Format::OggVorbis) => "audio/ogg",
        Some(Format::Wav) => "audio/wav",
        Some(Format::Aiff) => "audio/aiff",
        Some(Format::Aac | Format::Alac) => "audio/mp4",
        _ => "application/octet-stream",
    }
}

pub(crate) async fn respond(
    path: &Path,
    format: Option<Format>,
    headers: &HeaderMap,
) -> Result<Response, Failure> {
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|_| Failure::not_found("the track's file can't be opened"))?;
    let metadata = file.metadata().await.map_err(Failure::internal)?;
    let size = metadata.len();
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|time| time.as_millis() as i64)
        .unwrap_or_default();
    let etag = audio_etag(size, modified_ms);

    let header_text = |name| {
        headers
            .get(name)
            .and_then(|value: &HeaderValue| value.to_str().ok())
    };
    let current = header_text(header::IF_RANGE).is_none_or(|given| given == etag);
    let request = if current {
        parse_range(header_text(header::RANGE), size)
    } else {
        RangeRequest::Whole
    };

    let mut response_headers = HeaderMap::new();
    response_headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    response_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(content_type(format)),
    );
    if let Ok(value) = HeaderValue::from_str(&etag) {
        response_headers.insert(header::ETAG, value);
    }

    let (status, start, length) = match request {
        RangeRequest::Whole => (StatusCode::OK, 0, size),
        RangeRequest::Part(range) => {
            let value = format!("bytes {}-{}/{size}", range.start, range.end);
            if let Ok(value) = HeaderValue::from_str(&value) {
                response_headers.insert(header::CONTENT_RANGE, value);
            }
            (
                StatusCode::PARTIAL_CONTENT,
                range.start,
                range.end - range.start + 1,
            )
        }
        RangeRequest::Unsatisfiable => {
            let mut failure = Failure::new(
                ErrorCode::RangeNotSatisfiable,
                format!("the file has {size} bytes"),
            )
            .into_response();
            if let Ok(value) = HeaderValue::from_str(&format!("bytes */{size}")) {
                failure.headers_mut().insert(header::CONTENT_RANGE, value);
            }
            return Ok(failure);
        }
    };
    response_headers.insert(header::CONTENT_LENGTH, HeaderValue::from(length));
    if start > 0 {
        file.seek(SeekFrom::Start(start))
            .await
            .map_err(Failure::internal)?;
    }
    let body = Body::from_stream(ReaderStream::new(file.take(length)));
    Ok((status, response_headers, body).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(start: u64, end: u64) -> RangeRequest {
        RangeRequest::Part(ByteRange { start, end })
    }

    #[test]
    fn ranges_are_read_as_rfc_9110_describes_them() {
        assert_eq!(parse_range(None, 100), RangeRequest::Whole);
        assert_eq!(parse_range(Some("bytes=0-9"), 100), part(0, 9));
        assert_eq!(parse_range(Some("bytes=90-"), 100), part(90, 99));
        assert_eq!(parse_range(Some("bytes=90-500"), 100), part(90, 99));
        assert_eq!(parse_range(Some("bytes=-10"), 100), part(90, 99));
        assert_eq!(parse_range(Some("bytes=-500"), 100), part(0, 99));
        assert_eq!(
            parse_range(Some("bytes=100-"), 100),
            RangeRequest::Unsatisfiable
        );
        assert_eq!(
            parse_range(Some("bytes=-0"), 100),
            RangeRequest::Unsatisfiable
        );
        assert_eq!(
            parse_range(Some("bytes=0-"), 0),
            RangeRequest::Unsatisfiable
        );
        assert_eq!(parse_range(Some("bytes=9-0"), 100), RangeRequest::Whole);
        assert_eq!(parse_range(Some("bytes=0-1,5-6"), 100), RangeRequest::Whole);
        assert_eq!(parse_range(Some("items=0-1"), 100), RangeRequest::Whole);
        assert_eq!(parse_range(Some("bytes=a-b"), 100), RangeRequest::Whole);
    }
}
