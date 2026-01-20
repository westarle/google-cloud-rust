// Copyright 2025 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use super::Reader;
use super::parse_http_response;
use crate::model::ObjectChecksums;
use crate::model_ext::ObjectHighlights;


use crate::storage::checksum::details::Checksum as Hasher;
use crate::model::ObjectChecksums as Hashes;

fn hashes_from_highlights(highlights: &ObjectHighlights) -> Hashes {
    if let Some(checksums) = &highlights.checksums {
        checksums.clone()
    } else {
        Hashes::new()
    }
}

fn new_hasher(hashes: &Hashes) -> Hasher {
    Hasher {
        crc32c: hashes.crc32c.map(|_| Default::default()),
        md5_hash: if hashes.md5_hash.is_empty() {
            None
        } else {
            Some(Default::default())
        },
    }
}
use crate::storage::read_object::StorageHTTPResponse;
use crate::{Error, Result, error::ReadError};

/// A response to a [Storage::read_object] request.
#[derive(Debug)]
pub(crate) struct ResumableResponse {
    reader: Reader,
    response: Option<StorageHTTPResponse>,
    highlights: ObjectHighlights,
    hashes: Hashes,
    hasher: Hasher,
    offset: i64,
    limit: i64,
    last_seen_generation: i64,
    resume_count: u32,
}

impl ResumableResponse {
    pub(crate) fn new(reader: Reader, response: StorageHTTPResponse) -> Result<Self> {
        let mut generation = reader.request.generation;
        if generation == 0 {
            generation = crate::storage::read_object::parse_http_response::response_generation_from_headers(
                response.headers(),
            )
            .unwrap_or(0);
        }
        let highlights = crate::storage::read_object::parse_http_response::object_highlights(
            generation,
            response.headers(),
        )
        .map_err(Error::io)?;
        let hashes = hashes_from_highlights(&highlights);
        let hasher = new_hasher(&hashes);
        let offset = reader.request.read_offset;
        let mut limit = reader.request.read_limit;
        let last_seen_generation = highlights.generation;
        if limit == 0 && highlights.size > 0 {
            limit = highlights.size - offset;
        }
        // eprintln!("DEBUG: ResumableResponse::new limit={limit} highlights.size={} offset={offset}", highlights.size);

        Ok(Self {
            reader,
            response: Some(response),
            highlights,
            hashes,
            hasher,
            offset,
            limit,
            last_seen_generation,
            resume_count: 0,
        })
    }

    async fn next_attempt(&mut self) -> Result<StorageHTTPResponse> {
        loop {
            if self.response.is_some() {
                return Ok(self.response.take().unwrap());
            }
            let response = self.reader.read_with_retry().await;
            match response {
                Ok(r) => return Ok(r),
                Err(e) => self.resume(e).await?,
            }
        }
    }

    async fn next_chunk_from_response(&mut self) -> Result<Option<bytes::Bytes>> {
        let chunk = match &mut self.response {
            Some(StorageHTTPResponse::Legacy(r)) => r.chunk().await.map_err(Error::io)?,
            Some(StorageHTTPResponse::New { stream, .. }) => {
                use futures::StreamExt;
                match stream.next().await {
                    Some(Ok(bytes)) => Some(bytes),
                    Some(Err(e)) => return Err(Error::from(e)),
                    None => None,
                }
            }
            None => return Ok(None),
        };

        if let Some(data) = &chunk {
            self.hasher.update(self.offset as u64, data);
            self.offset += data.len() as i64;
            if self.limit > 0 {
                let len = data.len() as i64;
                if len > self.limit {
                    return Err(Error::io(ReadError::LongRead {
                        got: len as u64,
                        expected: self.limit as u64,
                    }));
                }
                self.limit -= len;
            }
        } else {
            if self.limit != 0 {
                return Err(Error::io(ReadError::ShortRead(self.limit as u64)));
            }
            if self.limit == 0 && self.offset < self.highlights.size {
                 return Err(Error::io(ReadError::ShortRead((self.highlights.size - self.offset) as u64)));
            }
            let computed = self.hasher.finalize();
            crate::storage::checksum::details::validate(&self.hashes, &Some(computed))
                .map_err(|e| Error::deser(ReadError::ChecksumMismatch(e)))?;
        }
        Ok(chunk)
    }

    async fn resume(&mut self, error: Error) -> Result<()> {
        use crate::read_resume_policy::{ResumeQuery, ResumeResult};

        // The existing read is no longer valid.
        self.response = None;
        self.resume_count += 1;
        let query = ResumeQuery::new(self.resume_count);
        match self
            .reader
            .options
            .read_resume_policy()
            .on_error(&query, error)
        {
            ResumeResult::Continue(_) => {}
            ResumeResult::Permanent(e) => return Err(e),
            ResumeResult::Exhausted(e) => return Err(e),
        };
        self.reader.request.read_offset = self.offset;
        self.reader.request.read_limit = self.limit;
        if self.limit == 0 && self.highlights.size > 0 {
            self.reader.request.read_limit = self.highlights.size - self.offset;
        }
        self.reader.request.generation = self.last_seen_generation;

        let response = self.reader.read_with_retry().await?;
        self.response = Some(response);
        Ok(())
    }
}

#[async_trait::async_trait]
impl crate::read_object::dynamic::ReadObjectResponse for ResumableResponse {
    fn object(&self) -> ObjectHighlights {
        self.highlights.clone()
    }

    async fn next(&mut self) -> Option<Result<bytes::Bytes>> {
        loop {
            // Ensure we have an active HTTP response stream.
            if self.response.is_none() {
                match self.next_attempt().await {
                    Ok(r) => self.response = Some(r),
                    Err(e) => return Some(Err(e)),
                }
            }

            match self.next_chunk_from_response().await {
                Ok(Some(b)) => return Some(Ok(b)),
                Ok(None) => return None,
                Err(e) => {
                    if let Err(resume_err) = self.resume(e).await {
                        return Some(Err(resume_err));
                    }
                }
            }
        }
    }
}

#[derive(Debug, PartialEq)]
struct ReadRange {
    start: u64,
    limit: u64,
}

fn response_range(response: &reqwest::Response) -> std::result::Result<ReadRange, ReadError> {
    match response.status() {
        reqwest::StatusCode::OK => {
            match (
                read_limit("content-length", response),
                read_limit("x-goog-stored-content-length", response),
            ) {
                (Ok(limit), _) => Ok(ReadRange { start: 0, limit }),
                (Err(_), Ok(limit)) => Ok(ReadRange { start: 0, limit }),
                (Err(e), Err(ReadError::MissingHeader(_))) => Err(e),
                (Err(ReadError::MissingHeader(_)), Err(e)) => Err(e),
                (Err(e), Err(_)) => Err(e),
            }
        }
        reqwest::StatusCode::PARTIAL_CONTENT => {
            let header = parse_http_response::required_header(response, "content-range")?;
            let header = header.strip_prefix("bytes ").ok_or_else(|| {
                ReadError::BadHeaderFormat("content-range", "missing bytes prefix".into())
            })?;
            let (range, _) = header.split_once('/').ok_or_else(|| {
                ReadError::BadHeaderFormat("content-range", "missing / separator".into())
            })?;
            let (start, end) = range.split_once('-').ok_or_else(|| {
                ReadError::BadHeaderFormat("content-range", "missing - separator".into())
            })?;
            let start = start
                .parse::<u64>()
                .map_err(|e| ReadError::BadHeaderFormat("content-range", e.into()))?;
            let end = end
                .parse::<u64>()
                .map_err(|e| ReadError::BadHeaderFormat("content-range", e.into()))?;
            // HTTP ranges are inclusive, we need to compute the number of bytes
            // in the range:
            let end = end + 1;
            let limit = end
                .checked_sub(start)
                .ok_or_else(|| ReadError::BadHeaderFormat("content-range", format!("range start ({start}) should be less than or equal to the range end ({end})").into()))?;
            Ok(ReadRange { start, limit })
        }
        s => Err(ReadError::UnexpectedSuccessCode(s.as_u16())),
    }
}

fn read_limit(
    name: &'static str,
    response: &reqwest::Response,
) -> std::result::Result<u64, ReadError> {
    let header = parse_http_response::required_header(response, name)?;
    header
        .parse::<u64>()
        .map_err(|e| ReadError::BadHeaderFormat(name, e.into()))
}

/// Returns the object checksums to validate against.
///
/// For some responses, the checksums are not expected to match the data.
/// The function returns an empty `ObjectChecksums` in such a case.
///
/// Checksum validation is supported iff:
/// 1. We requested the full content.
/// 2. We got all the content (status != PartialContent).
/// 3. The server sent a CRC header.
/// 4. The http stack did not uncompress the file.
/// 5. We were not served compressed data that was uncompressed on read.
///
/// For 4, we turn off automatic decompression in reqwest::Client when we
/// create it,
fn checksums_from_response(
    full_content_requested: bool,
    status: http::StatusCode,
    headers: &http::HeaderMap,
) -> ObjectChecksums {
    let checksums = ObjectChecksums::new();
    if !full_content_requested || status == http::StatusCode::PARTIAL_CONTENT {
        return checksums;
    }
    let stored_encoding = headers
        .get("x-goog-stored-content-encoding")
        .and_then(|e| e.to_str().ok())
        .map_or("", |e| e);
    let content_encoding = headers
        .get("content-encoding")
        .and_then(|e| e.to_str().ok())
        .map_or("", |e| e);
    if stored_encoding == "gzip" && content_encoding != "gzip" {
        return checksums;
    }
    checksums
        .set_or_clear_crc32c(parse_http_response::headers_to_crc32c(headers))
        .set_md5_hash(parse_http_response::headers_to_md5_hash(headers))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use std::error::Error as _;
    use test_case::test_case;

    type Result = anyhow::Result<()>;

    // The magic values are documented in super::tests::*.
    #[test_case(false, vec![("x-goog-hash", "crc32c=SZYC0g==,md5=d63R1fQSI9VYL8pzalyzNQ==")], http::StatusCode::OK, None, ""; "full content not requested")]
    #[test_case(true, vec![], http::StatusCode::PARTIAL_CONTENT, None, ""; "No x-goog-hash")]
    #[test_case(true, vec![("x-goog-hash", "crc32c=SZYC0g==,md5=d63R1fQSI9VYL8pzalyzNQ=="), ("x-goog-stored-content-encoding", "gzip"), ("content-encoding", "json")], http::StatusCode::OK, None, ""; "server uncompressed")]
    #[test_case(true, vec![("x-goog-hash", "crc32c=SZYC0g==,md5=d63R1fQSI9VYL8pzalyzNQ=="), ("x-goog-stored-content-encoding", "gzip"), ("content-encoding", "gzip")], http::StatusCode::OK, Some(1234567890_u32), "d63R1fQSI9VYL8pzalyzNQ=="; "both gzip")]
    #[test_case(true, vec![("x-goog-hash", "crc32c=SZYC0g==,md5=d63R1fQSI9VYL8pzalyzNQ==")], http::StatusCode::OK, Some(1234567890_u32), "d63R1fQSI9VYL8pzalyzNQ=="; "all ok")]
    fn test_checksums_validation_enabled(
        full_content_requested: bool,
        headers: Vec<(&str, &str)>,
        status: http::StatusCode,
        want_crc32c: Option<u32>,
        want_md5: &str,
    ) -> Result {
        let mut header_map = http::HeaderMap::new();
        for (key, value) in headers {
            header_map.insert(
                http::HeaderName::from_bytes(key.as_bytes())?,
                http::HeaderValue::from_bytes(value.as_bytes())?,
            );
        }

        let got = checksums_from_response(full_content_requested, status, &header_map);
        assert_eq!(got.crc32c, want_crc32c);
        assert_eq!(
            got.md5_hash,
            base64::prelude::BASE64_STANDARD.decode(want_md5)?
        );
        Ok(())
    }

    #[test_case("content-length", 0)]
    #[test_case("content-length", 1024)]
    #[test_case("x-goog-stored-content-length", 0)]
    #[test_case("x-goog-stored-content-length", 1024)]
    fn response_range_success(name: &'static str, limit: u64) -> Result {
        let response = http::Response::builder()
            .status(200)
            .header(name, limit)
            .body(Vec::new())?;
        let response = reqwest::Response::from(response);
        let range = response_range(&response)?;
        assert_eq!(range, super::ReadRange { start: 0, limit });
        Ok(())
    }

    #[test]
    fn response_range_missing() -> Result {
        let response = http::Response::builder().status(200).body(Vec::new())?;
        let response = reqwest::Response::from(response);
        let err = response_range(&response).expect_err("missing header should result in an error");
        assert!(
            matches!(err, ReadError::MissingHeader(h) if h == "content-length"),
            "{err:?}"
        );
        Ok(())
    }

    #[test_case("content-length", "")]
    #[test_case("content-length", "abc")]
    #[test_case("content-length", "-123")]
    #[test_case("x-goog-stored-content-length", "")]
    #[test_case("x-goog-stored-content-length", "abc")]
    #[test_case("x-goog-stored-content-length", "-123")]
    fn response_range_format(name: &'static str, value: &'static str) -> Result {
        let response = http::Response::builder()
            .status(200)
            .header(name, value)
            .body(Vec::new())?;
        let response = reqwest::Response::from(response);
        let err = response_range(&response).expect_err("header value should result in an error");
        assert!(
            matches!(err, ReadError::BadHeaderFormat(h, _) if h == name),
            "{err:?}"
        );
        assert!(err.source().is_some(), "{err:?}");
        Ok(())
    }

    #[test]
    fn response_range_format_both() -> Result {
        let response = http::Response::builder()
            .status(200)
            .header("content-length", "not-a-number")
            .header("x-goog-stored-content-length", "not-a-number")
            .body(Vec::new())?;
        let response = reqwest::Response::from(response);
        let err = response_range(&response).expect_err("header value should result in an error");
        assert!(matches!(err, ReadError::BadHeaderFormat(_, _)), "{err:?}");
        assert!(err.source().is_some(), "{err:?}");
        Ok(())
    }

    #[test_case(0, 123)]
    #[test_case(123, 456)]
    fn response_range_partial_success(start: u64, end: u64) -> Result {
        let response = http::Response::builder()
            .status(206)
            .header(
                "content-range",
                format!("bytes {}-{}/{}", start, end, end + 1),
            )
            .body(Vec::new())?;
        let response = reqwest::Response::from(response);
        let range = response_range(&response)?;
        assert_eq!(
            range,
            super::ReadRange {
                start,
                limit: (end + 1 - start)
            }
        );
        Ok(())
    }

    #[test]
    fn response_range_partial_missing() -> Result {
        let response = http::Response::builder().status(206).body(Vec::new())?;
        let response = reqwest::Response::from(response);
        let err = response_range(&response).expect_err("missing header should result in an error");
        assert!(
            matches!(err, ReadError::MissingHeader(h) if h == "content-range"),
            "{err:?}"
        );
        Ok(())
    }

    #[test_case("")]
    #[test_case("123-456/457"; "bad prefix")]
    #[test_case("bytes 123-456 457"; "bad separator")]
    #[test_case("bytes 123+456/457"; "bad separator [2]")]
    #[test_case("bytes abc-456/457"; "start is not numbers")]
    #[test_case("bytes 123-cde/457"; "end is not numbers")]
    #[test_case("bytes 123-0/457"; "invalid range")]
    fn response_range_partial_format(value: &'static str) -> Result {
        let response = http::Response::builder()
            .status(206)
            .header("content-range", value)
            .body(Vec::new())?;
        let response = reqwest::Response::from(response);
        let err = response_range(&response).expect_err("header value should result in an error");
        assert!(
            matches!(err, ReadError::BadHeaderFormat(h, _) if h == "content-range"),
            "{err:?}"
        );
        assert!(err.source().is_some(), "{err:?}");
        Ok(())
    }

    #[test]
    fn response_range_bad_response() -> Result {
        let code = reqwest::StatusCode::CREATED;
        let response = http::Response::builder().status(code).body(Vec::new())?;
        let response = reqwest::Response::from(response);
        let err = response_range(&response).expect_err("unexpected status creates error");
        assert!(
            matches!(err, ReadError::UnexpectedSuccessCode(c) if c == code),
            "{err:?}"
        );
        Ok(())
    }
}
