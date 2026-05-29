use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use tiktoken_rs::cl100k_base_singleton;

pub struct RawChunk {
    pub chunk_index: i32,
    pub content: String,
    pub heading_path: Option<String>,
    pub char_start: i32,
    pub char_end: i32,
    pub token_count: i32,
}

/// Split a Markdown document into chunks, optionally splitting at heading boundaries.
///
/// * `markdown` – source text (UTF-8)
/// * `max_tokens` – flush a chunk once its estimated token count exceeds this
/// * `min_tokens` – avoid flushing below this threshold (unless a heading forces a split)
/// * `split_by_heading` – whether to start a new chunk at qualifying headings
/// * `min_heading_level` / `max_heading_level` – inclusive range of heading levels that
///   trigger a split (1 = H1 … 6 = H6)
pub fn chunk_markdown(
    markdown: &str,
    max_tokens: i32,
    min_tokens: i32,
    split_by_heading: bool,
    min_heading_level: i32,
    max_heading_level: i32,
) -> Vec<RawChunk> {
    if markdown.is_empty() {
        return Vec::new();
    }

    let heading_map = build_heading_map(markdown);

    let parser = Parser::new_ext(markdown, Options::all());
    let events: Vec<(Event<'_>, std::ops::Range<usize>)> =
        parser.into_offset_iter().collect();

    let mut chunks: Vec<RawChunk> = Vec::new();
    let mut chunk_index: i32 = 0;
    let mut current_content = String::new();
    let mut chunk_byte_start: usize = 0;

    for (event, range) in &events {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                let level_u8 = *level as u8;

                // Decide whether this heading triggers a new chunk.
                let should_split = split_by_heading
                    && (level_u8 as i32) >= min_heading_level
                    && (level_u8 as i32) <= max_heading_level;

                if should_split && !current_content.is_empty() {
                    let tokens = count_tokens(&current_content);
                    if tokens > min_tokens {
                        flush_chunk(
                            &mut chunks,
                            &mut chunk_index,
                            &mut current_content,
                            markdown,
                            chunk_byte_start,
                            &heading_map,
                        );
                        chunk_byte_start = range.start;
                    }
                }

                // If this is the very first content, record start.
                if current_content.is_empty()
                    && chunk_byte_start == 0
                    && chunks.is_empty()
                {
                    chunk_byte_start = range.start;
                }

                // Append the heading marker text (the `#` markers are included in the
                // original source but not in the event text, so we reconstruct from
                // the source slice for fidelity).
                let source_slice = &markdown[range.start..range.end];
                current_content.push_str(source_slice);
            }

            Event::End(TagEnd::Heading(_)) => {}

            Event::Text(text) | Event::Code(text) => {
                current_content.push_str(text.as_ref());
            }

            Event::SoftBreak | Event::HardBreak => {
                current_content.push('\n');
            }

            Event::Html(html) => {
                current_content.push_str(html.as_ref());
            }

            Event::Start(tag) => {
                // For container tags that have source representations, include them.
                let rendered = render_start_tag(tag);
                current_content.push_str(&rendered);
            }

            Event::End(tag_end) => {
                let rendered = render_end_tag(tag_end);
                current_content.push_str(&rendered);
            }

            _ => {}
        }

        // Check size after accumulation.
        if !current_content.is_empty() {
            let tokens = count_tokens(&current_content);
            if tokens > max_tokens {
                flush_chunk(
                    &mut chunks,
                    &mut chunk_index,
                    &mut current_content,
                    markdown,
                    chunk_byte_start,
                    &heading_map,
                );
                // Next chunk starts after this event.
                chunk_byte_start = range.end;
            }
        }
    }

    // Flush remaining content.
    if !current_content.is_empty() {
        flush_chunk(
            &mut chunks,
            &mut chunk_index,
            &mut current_content,
            markdown,
            chunk_byte_start,
            &heading_map,
        );
    }

    chunks
}

fn flush_chunk(
    chunks: &mut Vec<RawChunk>,
    chunk_index: &mut i32,
    content: &mut String,
    markdown: &str,
    byte_start: usize,
    heading_map: &[(usize, u8, String)],
) {
    let char_start = byte_to_char(markdown, byte_start) as i32;
    let char_end = char_start + content.chars().count() as i32;
    let token_count = count_tokens(content);
    let heading_path = resolve_heading_path(byte_start, heading_map);

    chunks.push(RawChunk {
        chunk_index: *chunk_index,
        content: std::mem::take(content),
        heading_path,
        char_start,
        char_end,
        token_count,
    });
    *chunk_index += 1;
}

/// Build a sorted list of `(byte_offset, level, heading_text)` from the markdown.
fn build_heading_map(markdown: &str) -> Vec<(usize, u8, String)> {
    let parser = Parser::new_ext(markdown, Options::all());
    let events: Vec<(Event<'_>, std::ops::Range<usize>)> =
        parser.into_offset_iter().collect();

    let mut map: Vec<(usize, u8, String)> = Vec::new();
    let mut in_heading_level: Option<u8> = None;
    let mut heading_text = String::new();
    let mut heading_byte_start: usize = 0;

    for (event, range) in &events {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                in_heading_level = Some(*level as u8);
                heading_text.clear();
                heading_byte_start = range.start;
            }
            Event::Text(text) | Event::Code(text) => {
                if in_heading_level.is_some() {
                    if !heading_text.is_empty() {
                        heading_text.push(' ');
                    }
                    heading_text.push_str(text.as_ref());
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some(lvl) = in_heading_level.take() {
                    map.push((
                        heading_byte_start,
                        lvl,
                        std::mem::take(&mut heading_text),
                    ));
                }
            }
            _ => {}
        }
    }

    map
}

/// Given a byte offset, resolve the heading hierarchy path.
///
/// Walks the heading map, maintaining a stack. When a heading at level L is
/// encountered, all entries with level >= L are popped, then the current
/// heading is pushed. The result is the path joined with " > ".
fn resolve_heading_path(
    byte_offset: usize,
    heading_map: &[(usize, u8, String)],
) -> Option<String> {
    // Stack of (level, text).
    let mut stack: Vec<(u8, String)> = Vec::new();

    for &(offset, level, ref text) in heading_map {
        if offset > byte_offset {
            break;
        }
        // Pop entries at >= current level.
        while stack.last().is_some_and(|(l, _)| *l >= level) {
            stack.pop();
        }
        stack.push((level, text.clone()));
    }

    if stack.is_empty() {
        return None;
    }

    let path: Vec<&str> = stack.iter().map(|(_, t)| t.as_str()).collect();
    Some(path.join(" > "))
}

/// Convert a byte offset to a character offset within `text`.
fn byte_to_char(text: &str, byte_offset: usize) -> usize {
    if byte_offset == 0 {
        return 0;
    }
    let end = byte_offset.min(text.len());
    if end == 0 {
        return 0;
    }
    // Find the nearest valid UTF-8 boundary at or before `end`.
    let mut boundary = end;
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    text[..boundary].chars().count()
}

/// Count tokens using the cl100k_base tokenizer.
fn count_tokens(text: &str) -> i32 {
    let bpe = cl100k_base_singleton();
    let locked = bpe.lock();
    locked.encode_with_special_tokens(text).len() as i32
}

/// Render a start tag back to Markdown-ish source text.
fn render_start_tag(tag: &Tag<'_>) -> String {
    match tag {
        Tag::Paragraph => String::new(),
        Tag::BlockQuote(_) => "> ".to_string(),
        Tag::CodeBlock(kind) => {
            let lang = match kind {
                pulldown_cmark::CodeBlockKind::Fenced(info) => info.to_string(),
                _ => String::new(),
            };
            format!("```{lang}\n")
        }
        Tag::List(start) => {
            if let Some(n) = start {
                format!("{n}. ")
            } else {
                "- ".to_string()
            }
        }
        Tag::Item => String::new(),
        Tag::Emphasis => "*".to_string(),
        Tag::Strong => "**".to_string(),
        Tag::Strikethrough => "~~".to_string(),
        Tag::Link {
            dest_url, title, ..
        } => {
            if title.is_empty() {
                format!("[{}]({})", "", dest_url)
            } else {
                format!("[{}]({} \"{}\")", "", dest_url, title)
            }
        }
        Tag::Image {
            dest_url, title, ..
        } => {
            if title.is_empty() {
                format!("![{}]({})", "", dest_url)
            } else {
                format!("![{}]({} \"{}\")", "", dest_url, title)
            }
        }
        _ => String::new(),
    }
}

/// Render an end tag back to Markdown-ish source text.
fn render_end_tag(tag_end: &TagEnd) -> String {
    match tag_end {
        TagEnd::Paragraph => "\n\n".to_string(),
        TagEnd::Heading(_) => "\n".to_string(),
        TagEnd::BlockQuote(_) => "\n".to_string(),
        TagEnd::CodeBlock => "```\n".to_string(),
        TagEnd::Emphasis => "*".to_string(),
        TagEnd::Strong => "**".to_string(),
        TagEnd::Strikethrough => "~~".to_string(),
        TagEnd::Link | TagEnd::Image => String::new(),
        TagEnd::Item => "\n".to_string(),
        TagEnd::List(_) => "\n".to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_markdown_empty_input() {
        let chunks = chunk_markdown("", 800, 100, true, 1, 4);
        assert!(chunks.is_empty());
    }

    #[test]
    fn chunk_markdown_single_paragraph() {
        let md = "Hello world, this is a simple paragraph.";
        let chunks = chunk_markdown(md, 800, 100, true, 1, 4);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_index, 0);
        assert!(chunks[0].content.contains("Hello world"));
        assert!(chunks[0].heading_path.is_none());
    }

    #[test]
    fn chunk_markdown_split_by_heading() {
        let md = "\
# Chapter 1

Content of chapter one.

# Chapter 2

Content of chapter two.";

        let chunks = chunk_markdown(md, 800, 10, true, 1, 4);
        assert!(
            chunks.len() >= 2,
            "Should split into at least 2 chunks at H1 headings, got {}",
            chunks.len()
        );
    }

    #[test]
    fn chunk_markdown_heading_path() {
        let md = "\
# Top Level

Some intro text here that provides enough content for the first chunk to be split before the second heading.

## Sub Section

Some detail text here that adds more content under the sub section heading.";

        let chunks = chunk_markdown(md, 20, 5, true, 1, 4);
        // With low max_tokens, should split at headings
        let sub_chunk = chunks.iter().find(|c| {
            c.heading_path
                .as_ref()
                .is_some_and(|p| p.contains("Sub Section"))
        });
        assert!(
            sub_chunk.is_some(),
            "Should find a chunk with heading path containing 'Sub Section', chunks: {:?}",
            chunks.iter().map(|c| &c.heading_path).collect::<Vec<_>>()
        );
    }

    #[test]
    fn chunk_markdown_no_heading_split_when_disabled() {
        let md = "\
# Chapter 1

Content one.

# Chapter 2

Content two.";

        let chunks = chunk_markdown(md, 800, 100, false, 1, 4);
        assert_eq!(
            chunks.len(),
            1,
            "With split_by_heading=false, should produce single chunk"
        );
    }

    #[test]
    fn chunk_markdown_respects_heading_level_range() {
        let paragraph = "This is some content that makes the chunk large enough to be worth splitting. ".repeat(5);
        let md = format!(
            "\
# H1

{paragraph}

### H3

{paragraph}

# Another H1

{paragraph}"
        );

        // Only split on H3 (level 3), not H1 (level 1)
        let chunks = chunk_markdown(&md, 30, 5, true, 3, 3);
        // H1 headings should NOT trigger splits since they're outside [3,3]
        // H3 heading SHOULD trigger a split
        assert!(
            chunks.len() >= 2,
            "Should split at H3 but not H1, got {} chunks",
            chunks.len()
        );
    }

    #[test]
    fn chunk_markdown_token_count_positive() {
        let md = "This is a simple test document with some text.";
        let chunks = chunk_markdown(md, 800, 100, true, 1, 4);
        assert_eq!(chunks.len(), 1);
        assert!(
            chunks[0].token_count > 0,
            "Token count should be positive for non-empty content"
        );
    }

    #[test]
    fn chunk_markdown_char_offsets_valid() {
        let md = "# Intro\n\nHello world.\n\n## Details\n\nMore text.";
        let chunks = chunk_markdown(md, 800, 10, true, 1, 4);
        for chunk in &chunks {
            assert!(
                chunk.char_start >= 0,
                "char_start should be >= 0, got {}",
                chunk.char_start
            );
            assert!(
                chunk.char_end >= chunk.char_start,
                "char_end should be >= char_start"
            );
        }
    }

    #[test]
    fn chunk_markdown_large_document_produces_multiple_chunks() {
        // Build a large document that exceeds max_tokens
        let paragraph = "This is a paragraph with some content. ".repeat(100);
        let md = format!("# Title\n\n{paragraph}");
        let chunks = chunk_markdown(&md, 50, 10, true, 1, 4);
        assert!(
            chunks.len() > 1,
            "Large document should produce multiple chunks"
        );
        // Verify chunk indices are sequential
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(
                chunk.chunk_index, i as i32,
                "Chunk index should be sequential"
            );
        }
    }

    #[test]
    fn chunk_markdown_code_block_preserved() {
        let md = "\
# Code Example

```rust
fn main() {
    println!(\"Hello\");
}
```";

        let chunks = chunk_markdown(md, 800, 10, true, 1, 4);
        assert!(!chunks.is_empty());
        let combined: String = chunks.iter().map(|c| c.content.as_str()).collect();
        assert!(
            combined.contains("fn main"),
            "Code block content should be preserved in chunks"
        );
    }

    #[test]
    fn chunk_markdown_chinese_content() {
        let md = "# 中文标题\n\n这是中文内容，用于测试分块功能是否正常工作。";
        let chunks = chunk_markdown(md, 800, 10, true, 1, 4);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].content.contains("中文内容"));
    }

    // ── helper function tests ────────────────────────────────────

    #[test]
    fn byte_to_char_start_of_string() {
        assert_eq!(byte_to_char("hello", 0), 0);
    }

    #[test]
    fn byte_to_char_mid_ascii() {
        assert_eq!(byte_to_char("hello", 3), 3);
    }

    #[test]
    fn byte_to_char_unicode() {
        let text = "你好世界";
        // Each CJK char is 3 bytes in UTF-8
        // byte offset 3 = char offset 1
        assert_eq!(byte_to_char(text, 3), 1);
        assert_eq!(byte_to_char(text, 6), 2);
    }

    #[test]
    fn byte_to_char_beyond_end() {
        let result = byte_to_char("abc", 999);
        assert_eq!(result, 3, "Should clamp to text length");
    }

    #[test]
    fn count_tokens_returns_positive_for_non_empty() {
        let count = count_tokens("Hello, world!");
        assert!(count > 0);
    }

    #[test]
    fn count_tokens_empty_string() {
        let count = count_tokens("");
        assert_eq!(count, 0);
    }

    #[test]
    fn resolve_heading_path_empty_map() {
        let result = resolve_heading_path(0, &[]);
        assert!(result.is_none());
    }

    #[test]
    fn resolve_heading_path_single_heading() {
        let map = vec![(0usize, 1u8, "Title".to_string())];
        let result = resolve_heading_path(50, &map);
        assert_eq!(result.as_deref(), Some("Title"));
    }

    #[test]
    fn resolve_heading_path_nested() {
        let map = vec![
            (0usize, 1u8, "Chapter".to_string()),
            (100usize, 2u8, "Section".to_string()),
        ];
        let result = resolve_heading_path(150, &map);
        assert_eq!(result.as_deref(), Some("Chapter > Section"));
    }

    #[test]
    fn resolve_heading_path_before_first_heading() {
        let map = vec![(50usize, 1u8, "Chapter".to_string())];
        let result = resolve_heading_path(10, &map);
        assert!(result.is_none());
    }

    #[test]
    fn build_heading_map_extracts_headings() {
        let md = "# Title\n\n## Sub\n\n### Deep\n";
        let map = build_heading_map(md);
        assert_eq!(map.len(), 3);
        assert_eq!(map[0].1, 1);
        assert_eq!(map[0].2, "Title");
        assert_eq!(map[1].1, 2);
        assert_eq!(map[1].2, "Sub");
        assert_eq!(map[2].1, 3);
        assert_eq!(map[2].2, "Deep");
    }
}
