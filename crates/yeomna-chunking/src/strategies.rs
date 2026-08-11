//! Chunking strategy implementations.

use super::{ChunkingStrategy, TextChunk};

// ---------------------------------------------------------------------------
// Token-based chunking
// ---------------------------------------------------------------------------

/// Splits text into chunks of approximately `chunk_size` whitespace-delimited
/// tokens with `overlap` tokens of overlap between consecutive chunks.
///
/// Uses a simple whitespace tokenizer.  For ML-accurate token counts, the
/// Persephone service handles BPE tokenization; this is a fast, deterministic
/// approximation suitable for chunk boundary calculation.
pub struct TokenChunking {
    /// Target tokens per chunk.
    pub chunk_size: usize,
    /// Overlap tokens between consecutive chunks.
    pub overlap: usize,
}

impl Default for TokenChunking {
    fn default() -> Self {
        Self {
            chunk_size: 500,
            overlap: 200,
        }
    }
}

impl ChunkingStrategy for TokenChunking {
    fn chunk(&self, text: &str) -> Vec<TextChunk> {
        if text.is_empty() {
            return Vec::new();
        }

        // Collect (byte_start, byte_end) for each whitespace-delimited token.
        let tokens: Vec<(usize, usize)> = token_spans(text);
        if tokens.is_empty() {
            return Vec::new();
        }

        if self.chunk_size == 0 {
            return Vec::new();
        }

        let step = self.chunk_size.saturating_sub(self.overlap).max(1);
        let mut chunks = Vec::new();
        let mut start = 0;

        while start < tokens.len() {
            let end = (start + self.chunk_size).min(tokens.len());
            let char_start = tokens[start].0;
            let char_end = tokens[end - 1].1;

            chunks.push(TextChunk {
                text: text[char_start..char_end].to_string(),
                start_char: char_start,
                end_char: char_end,
                chunk_index: chunks.len(),
                total_chunks: 0, // filled below
            });

            if end >= tokens.len() {
                break;
            }
            start += step;
        }

        let total = chunks.len();
        for c in &mut chunks {
            c.total_chunks = total;
        }
        chunks
    }
}

// ---------------------------------------------------------------------------
// Sliding window chunking (character-based)
// ---------------------------------------------------------------------------

/// Sliding window chunking with configurable window size and step.
///
/// Window and step sizes are measured in Unicode characters.  The resulting
/// [`TextChunk`] offsets are byte offsets (consistent with other strategies).
pub struct SlidingWindowChunking {
    /// Window size in characters.
    pub window_size: usize,
    /// Step size in characters (window_size - overlap).
    pub step_size: usize,
}

impl Default for SlidingWindowChunking {
    fn default() -> Self {
        Self {
            window_size: 512,
            step_size: 256,
        }
    }
}

impl ChunkingStrategy for SlidingWindowChunking {
    fn chunk(&self, text: &str) -> Vec<TextChunk> {
        if text.is_empty() || self.window_size == 0 {
            return Vec::new();
        }

        let step = self.step_size.max(1);
        let mut chunks = Vec::new();

        // Work on character indices for correct Unicode handling.
        let char_indices: Vec<(usize, char)> = text.char_indices().collect();
        let total_chars = char_indices.len();
        let mut char_pos = 0;

        while char_pos < total_chars {
            let end_char_pos = (char_pos + self.window_size).min(total_chars);
            let byte_start = char_indices[char_pos].0;
            let byte_end = if end_char_pos < total_chars {
                char_indices[end_char_pos].0
            } else {
                text.len()
            };

            chunks.push(TextChunk {
                text: text[byte_start..byte_end].to_string(),
                start_char: byte_start,
                end_char: byte_end,
                chunk_index: chunks.len(),
                total_chunks: 0,
            });

            if end_char_pos >= total_chars {
                break;
            }
            char_pos += step;
        }

        let total = chunks.len();
        for c in &mut chunks {
            c.total_chunks = total;
        }
        chunks
    }
}

// ---------------------------------------------------------------------------
// Sentence-based chunking
// ---------------------------------------------------------------------------

/// Splits text on sentence boundaries, grouping sentences into chunks
/// that don't exceed `max_chunk_size` characters.
///
/// Sizes are measured in Unicode characters over the full chunk span,
/// including the whitespace between sentences. A single sentence longer
/// than `max_chunk_size` stays whole rather than being split mid-sentence.
pub struct SentenceChunking {
    /// Maximum characters per chunk.
    pub max_chunk_size: usize,
    /// Minimum characters per chunk (avoids tiny trailing chunks).
    pub min_chunk_size: usize,
}

impl Default for SentenceChunking {
    fn default() -> Self {
        Self {
            max_chunk_size: 1500,
            min_chunk_size: 100,
        }
    }
}

impl ChunkingStrategy for SentenceChunking {
    fn chunk(&self, text: &str) -> Vec<TextChunk> {
        if text.is_empty() {
            return Vec::new();
        }

        let sentences = split_sentences(text);
        if sentences.is_empty() {
            return Vec::new();
        }

        let mut chunks = Vec::new();
        let mut current_start: usize = sentences[0].0;
        let mut current_end: usize = sentences[0].0;
        // Characters in `text[current_start..current_end]`, tracked
        // incrementally so the loop stays linear in document length.
        let mut current_chars: usize = 0;

        for &(sent_start, sent_end) in &sentences {
            let sent_chars = text[sent_start..sent_end].chars().count();

            if current_chars > 0 {
                // Candidate length if this sentence joins the current chunk:
                // the full span in characters, including the whitespace gap
                // between the current chunk and this sentence.
                let gap_chars = text[current_end..sent_start].chars().count();
                let combined = current_chars + gap_chars + sent_chars;

                if combined > self.max_chunk_size {
                    // Flush current chunk.
                    chunks.push(TextChunk {
                        text: text[current_start..current_end].to_string(),
                        start_char: current_start,
                        end_char: current_end,
                        chunk_index: chunks.len(),
                        total_chunks: 0,
                    });
                    current_start = sent_start;
                    current_chars = sent_chars;
                } else {
                    current_chars = combined;
                }
            } else {
                current_chars = sent_chars;
            }
            current_end = sent_end;
        }

        // Flush remaining text.
        if current_end > current_start {
            // Merge a tiny trailing chunk into the previous one, but only
            // when the merged chunk still respects `max_chunk_size`.
            let merge_ok = current_chars < self.min_chunk_size
                && chunks.last().is_some_and(|last| {
                    text[last.start_char..current_end].chars().count() <= self.max_chunk_size
                });
            if merge_ok {
                let last = chunks.last_mut().unwrap();
                last.text = text[last.start_char..current_end].to_string();
                last.end_char = current_end;
            } else {
                chunks.push(TextChunk {
                    text: text[current_start..current_end].to_string(),
                    start_char: current_start,
                    end_char: current_end,
                    chunk_index: chunks.len(),
                    total_chunks: 0,
                });
            }
        }

        let total = chunks.len();
        for c in &mut chunks {
            c.total_chunks = total;
        }
        chunks
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return byte-offset spans for whitespace-delimited tokens.
fn token_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut in_token = false;
    let mut start = 0;

    for (i, ch) in text.char_indices() {
        if ch.is_whitespace() {
            if in_token {
                spans.push((start, i));
                in_token = false;
            }
        } else if !in_token {
            start = i;
            in_token = true;
        }
    }
    if in_token {
        spans.push((start, text.len()));
    }
    spans
}

/// Simple sentence splitter based on punctuation followed by whitespace.
///
/// Returns byte-offset spans `(start, end)` for each sentence.
fn split_sentences(text: &str) -> Vec<(usize, usize)> {
    let mut sentences = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Sentence-ending punctuation followed by whitespace or end-of-text.
        if matches!(bytes[i], b'.' | b'!' | b'?') {
            let after = i + 1;
            if after >= len || bytes[after].is_ascii_whitespace() {
                let end = after.min(len);
                // Include trailing whitespace in the sentence span
                // so the next sentence starts at a non-space character.
                let mut trim_end = end;
                while trim_end < len && bytes[trim_end].is_ascii_whitespace() {
                    trim_end += 1;
                }
                sentences.push((start, end));
                start = trim_end;
                i = trim_end;
                continue;
            }
        }
        i += 1;
    }

    // Trailing text without sentence-ending punctuation.
    if start < len {
        sentences.push((start, len));
    }

    sentences
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Token chunking ---------------------------------------------------

    #[test]
    fn test_token_chunking_basic() {
        let text = "one two three four five six seven eight nine ten";
        let chunker = TokenChunking {
            chunk_size: 4,
            overlap: 2,
        };
        let chunks = chunker.chunk(text);

        assert!(chunks.len() >= 2);
        // First chunk has 4 tokens
        assert_eq!(chunks[0].text, "one two three four");
        assert_eq!(chunks[0].chunk_index, 0);
        assert_eq!(chunks[0].start_char, 0);
        // Overlap: step = 4-2 = 2, so second chunk starts at token index 2
        assert_eq!(chunks[1].text, "three four five six");
        // All chunks know the total
        for c in &chunks {
            assert_eq!(c.total_chunks, chunks.len());
        }
    }

    #[test]
    fn test_token_chunking_empty() {
        let chunker = TokenChunking::default();
        assert!(chunker.chunk("").is_empty());
        assert!(chunker.chunk("   ").is_empty());
    }

    #[test]
    fn test_token_chunking_single_chunk() {
        let text = "hello world";
        let chunker = TokenChunking {
            chunk_size: 100,
            overlap: 10,
        };
        let chunks = chunker.chunk(text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, "hello world");
        assert_eq!(chunks[0].total_chunks, 1);
    }

    // -- Sliding window ---------------------------------------------------

    #[test]
    fn test_sliding_window_basic() {
        let text = "abcdefghij"; // 10 chars
        let chunker = SlidingWindowChunking {
            window_size: 4,
            step_size: 2,
        };
        let chunks = chunker.chunk(text);
        assert_eq!(chunks[0].text, "abcd");
        assert_eq!(chunks[1].text, "cdef");
        assert!(chunks.len() >= 3);
    }

    #[test]
    fn test_sliding_window_empty() {
        let chunker = SlidingWindowChunking::default();
        assert!(chunker.chunk("").is_empty());
    }

    // -- Sentence chunking ------------------------------------------------

    #[test]
    fn test_sentence_chunking_basic() {
        let text = "First sentence. Second sentence. Third sentence.";
        let chunker = SentenceChunking {
            max_chunk_size: 35,
            min_chunk_size: 5,
        };
        let chunks = chunker.chunk(text);
        assert!(chunks.len() >= 2);
        // First chunk should contain at least the first sentence
        assert!(chunks[0].text.contains("First"));
    }

    #[test]
    fn test_sentence_chunking_tiny_trailing_not_merged_over_max() {
        // "OK." (3 chars) is below min_chunk_size, but merging it would make
        // the previous chunk 32 chars, over max_chunk_size = 30. The merge
        // must be refused and the tiny trailing chunk kept separate.
        let text = "A long enough sentence here. OK.";
        let chunker = SentenceChunking {
            max_chunk_size: 30,
            min_chunk_size: 10,
        };
        let chunks = chunker.chunk(text);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[1].text, "OK.");
        for c in &chunks {
            assert!(c.text.chars().count() <= 30);
        }
    }

    #[test]
    fn test_sentence_chunking_tiny_trailing_merged_within_max() {
        // Same text with a larger max: the merge fits, so it happens.
        let text = "A long enough sentence here. OK.";
        let chunker = SentenceChunking {
            max_chunk_size: 40,
            min_chunk_size: 10,
        };
        let chunks = chunker.chunk(text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].text, text);
    }

    #[test]
    fn test_sentence_chunking_counts_chars_not_bytes() {
        // Two sentences, 11 characters over the combined span but 19 bytes.
        // A max of 12 fits the character count, so this must stay one chunk.
        // Byte-based counting would split it.
        let text = "\u{e9}\u{e9}\u{e9}\u{e9}. \u{e9}\u{e9}\u{e9}\u{e9}.";
        assert_eq!(text.chars().count(), 11);
        assert_eq!(text.len(), 19);
        let chunker = SentenceChunking {
            max_chunk_size: 12,
            min_chunk_size: 1,
        };
        let chunks = chunker.chunk(text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(
            &text[chunks[0].start_char..chunks[0].end_char],
            chunks[0].text
        );
    }

    #[test]
    fn test_sentence_chunking_exact_max_boundary() {
        // Combined span of exactly max_chunk_size characters is allowed.
        // "One two. Four five." is 19 chars.
        let text = "One two. Four five.";
        assert_eq!(text.chars().count(), 19);
        let chunker = SentenceChunking {
            max_chunk_size: 19,
            min_chunk_size: 1,
        };
        let chunks = chunker.chunk(text);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn test_sentence_chunking_empty() {
        let chunker = SentenceChunking::default();
        assert!(chunker.chunk("").is_empty());
    }

    // -- Helpers ----------------------------------------------------------

    #[test]
    fn test_token_spans() {
        let spans = token_spans("hello  world  foo");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0], (0, 5)); // "hello"
        assert_eq!(spans[1], (7, 12)); // "world"
        assert_eq!(spans[2], (14, 17)); // "foo"
    }

    #[test]
    fn test_split_sentences() {
        let text = "Hello world. This is a test! And more? Final.";
        let sentences = split_sentences(text);
        assert_eq!(sentences.len(), 4);
        assert_eq!(&text[sentences[0].0..sentences[0].1], "Hello world.");
        assert_eq!(&text[sentences[1].0..sentences[1].1], "This is a test!");
    }

    // -- Offsets are correct ------------------------------------------------

    #[test]
    fn test_chunk_offsets_roundtrip() {
        let text = "The quick brown fox jumps over the lazy dog and more words here";
        let chunker = TokenChunking {
            chunk_size: 3,
            overlap: 1,
        };
        let chunks = chunker.chunk(text);
        for c in &chunks {
            assert_eq!(&text[c.start_char..c.end_char], c.text);
        }
    }

    #[test]
    fn test_token_chunking_zero_chunk_size() {
        let chunker = TokenChunking {
            chunk_size: 0,
            overlap: 0,
        };
        assert!(chunker.chunk("hello world").is_empty());
    }

    // -- Unicode regression -------------------------------------------------

    #[test]
    fn test_unicode_byte_offsets_token() {
        // "café bon" — 'é' is 2 bytes in UTF-8
        let text = "café bon jour ici";
        let chunker = TokenChunking {
            chunk_size: 2,
            overlap: 0,
        };
        let chunks = chunker.chunk(text);
        for c in &chunks {
            assert_eq!(&text[c.start_char..c.end_char], c.text);
        }
    }

    #[test]
    fn test_unicode_byte_offsets_sliding_window() {
        // Each emoji is 4 bytes. "🔥🌊🎉" = 12 bytes, 3 chars.
        let text = "🔥🌊🎉ab";
        let chunker = SlidingWindowChunking {
            window_size: 2,
            step_size: 1,
        };
        let chunks = chunker.chunk(text);
        for c in &chunks {
            // Byte offsets must slice correctly
            assert_eq!(&text[c.start_char..c.end_char], c.text);
        }
    }

    #[test]
    fn test_unicode_byte_offsets_sentence() {
        let text = "Ünited Stätes. Bönn city.";
        let chunker = SentenceChunking {
            max_chunk_size: 20,
            min_chunk_size: 5,
        };
        let chunks = chunker.chunk(text);
        for c in &chunks {
            assert_eq!(&text[c.start_char..c.end_char], c.text);
        }
    }
}
