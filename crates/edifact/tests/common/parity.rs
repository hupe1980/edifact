//! Chunking readers and the full-`Result` parity check between the slice and
//! reader parse paths.
//!
//! Shared by `parse_path_parity.rs` (class regression tests and the exhaustive
//! sweep) and the `fuzz_parity_*` bolero targets. Included with
//! `#[path = "common/parity.rs"] mod parity;`, so each suite compiles only what
//! it uses.
#![allow(dead_code)]

use edifact::{
    EdifactError, OwnedSegment, ReaderConfig, from_bufread_with_config,
    from_bytes_decoded_with_config, from_bytes_with_config, from_reader_decoded_with_config,
};
use std::io::{BufRead, BufReader, Read};

/// The whole observable answer of one parse.
pub type ParseResult = Result<Vec<OwnedSegment>, EdifactError>;

/// Buffer sizes every reader-side check runs with.
pub const CHUNKS: [usize; 6] = [1, 2, 3, 7, 8, 8192];

/// A `BufRead` over a byte slice whose `fill_buf` returns at most `chunk`
/// bytes, whatever the caller consumed before.
pub struct ChunkedBufRead<'a> {
    data: &'a [u8],
    pos: usize,
    chunk: usize,
}

impl<'a> ChunkedBufRead<'a> {
    pub fn new(data: &'a [u8], chunk: usize) -> Self {
        assert!(chunk > 0, "a zero-byte fill would read as end of input");
        Self {
            data,
            pos: 0,
            chunk,
        }
    }
}

impl Read for ChunkedBufRead<'_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let available = self.fill_buf()?;
        let n = available.len().min(out.len());
        out[..n].copy_from_slice(&available[..n]);
        self.consume(n);
        Ok(n)
    }
}

impl BufRead for ChunkedBufRead<'_> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        let end = self.pos.saturating_add(self.chunk).min(self.data.len());
        Ok(&self.data[self.pos..end])
    }

    fn consume(&mut self, amt: usize) {
        self.pos = (self.pos + amt).min(self.data.len());
    }
}

/// A `Read` that hands out a pseudo-random 1–3 bytes per call (seeded
/// xorshift), the way a slow socket does.
pub struct TrickleRead<'a> {
    data: &'a [u8],
    pos: usize,
    state: u64,
}

impl<'a> TrickleRead<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            state: 0x9E37_79B9_7F4A_7C15,
        }
    }
}

impl Read for TrickleRead<'_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        let want = 1 + (self.state % 3) as usize;
        let n = want.min(out.len()).min(self.data.len() - self.pos);
        out[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// `from_bytes_with_config`, with the segments made owned.
pub fn slice_result(input: &[u8], config: ReaderConfig) -> ParseResult {
    from_bytes_with_config(input, config)
        .map(|r| r.map(edifact::Segment::into_owned))
        .collect()
}

/// `from_bufread_with_config` under every chunking in [`CHUNKS`] and behind a
/// [`TrickleRead`], labelled.
pub fn reader_results(input: &[u8], config: ReaderConfig) -> Vec<(String, ParseResult)> {
    let mut results: Vec<(String, ParseResult)> = CHUNKS
        .iter()
        .map(|&chunk| {
            (
                format!("chunk {chunk}"),
                from_bufread_with_config(ChunkedBufRead::new(input, chunk), config).collect(),
            )
        })
        .collect();
    results.push((
        "trickle".to_owned(),
        from_bufread_with_config(BufReader::new(TrickleRead::new(input)), config).collect(),
    ));
    results
}

/// The slice result and every reader result for `input`.
pub fn parse_both(input: &[u8], config: ReaderConfig) -> (ParseResult, Vec<(String, ParseResult)>) {
    (slice_result(input, config), reader_results(input, config))
}

/// Assert that every reader result equals the slice result, printing the
/// input and all results on failure.
pub fn assert_full_parity(input: &[u8], config: ReaderConfig) {
    let (slice, readers) = parse_both(input, config);
    for (label, reader) in &readers {
        assert_eq!(
            reader,
            &slice,
            "slice and reader ({label}) disagree on {:?} under {config:?}",
            String::from_utf8_lossy(input),
        );
    }
}

/// Assert the decoded pair agrees: `from_bytes_decoded_with_config` against
/// `from_reader_decoded_with_config` behind several chunkings.
pub fn assert_decoded_parity(input: &[u8], config: ReaderConfig) {
    let slice = from_bytes_decoded_with_config(input, config);
    let readers: [(&str, ParseResult); 3] = [
        (
            "chunk 1",
            from_reader_decoded_with_config(ChunkedBufRead::new(input, 1), config).collect(),
        ),
        (
            "chunk 7",
            from_reader_decoded_with_config(ChunkedBufRead::new(input, 7), config).collect(),
        ),
        (
            "trickle",
            from_reader_decoded_with_config(TrickleRead::new(input), config).collect(),
        ),
    ];
    for (label, reader) in &readers {
        assert_eq!(
            reader,
            &slice,
            "decoded slice and reader ({label}) disagree on {:?} under {config:?}",
            String::from_utf8_lossy(input),
        );
    }
}
