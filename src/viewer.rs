use crate::{Block, ContentFormat};
use console::measure_text_width;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineKind {
    Text,
    Heading,
    Code,
    Quote,
    Rule,
}
pub(crate) struct Line {
    pub text: String,
    pub source: usize,
    pub kind: LineKind,
}

/// A deliberately small terminal Markdown presentation, with one source mapping
/// for every wrapped row. Unsupported syntax stays readable as ordinary text.
pub(crate) fn layout(block: &Block, raw: bool, width: usize) -> Vec<Line> {
    let markdown = block.format == ContentFormat::Markdown && !raw;
    let mut fence: Option<char> = None;
    let mut rows = Vec::new();
    for (source, line) in block.content.lines().enumerate() {
        let safe: String = line
            .replace('\t', "    ")
            .chars()
            .map(|c| if c.is_control() { '�' } else { c })
            .collect();
        let (text, kind) = if markdown {
            markdown_line(&safe, &mut fence)
        } else {
            (safe, LineKind::Text)
        };
        for text in wrap(&text, width.max(1)) {
            rows.push(Line { text, source, kind });
        }
    }
    rows
}
fn markdown_line(line: &str, fence: &mut Option<char>) -> (String, LineKind) {
    let trimmed = line.trim_start();
    let marker = if trimmed.starts_with("```") {
        Some('`')
    } else if trimmed.starts_with("~~~") {
        Some('~')
    } else {
        None
    };
    if let Some(active) = *fence {
        if marker == Some(active) && trimmed.trim_matches(active).trim().is_empty() {
            *fence = None;
            return ("────────────────".into(), LineKind::Rule);
        }
        return (line.into(), LineKind::Code);
    }
    if let Some(marker) = marker {
        *fence = Some(marker);
        let language = trimmed.trim_start_matches(marker).trim();
        return (
            format!(
                "Code{}",
                if language.is_empty() {
                    String::new()
                } else {
                    format!(" · {language}")
                }
            ),
            LineKind::Rule,
        );
    }
    let hashes = trimmed.bytes().take_while(|byte| *byte == b'#').count();
    if (1..=6).contains(&hashes) && trimmed.as_bytes().get(hashes) == Some(&b' ') {
        let heading = trimmed[hashes..].trim();
        let without_hashes = heading.trim_end_matches('#');
        let heading = if without_hashes.ends_with(' ') {
            without_hashes.trim_end()
        } else {
            heading
        };
        return (inline(heading), LineKind::Heading);
    }
    if matches!(trimmed.trim(), "---" | "***" | "___") {
        return ("────────────────".into(), LineKind::Rule);
    }
    if let Some(quote) = trimmed.strip_prefix("> ") {
        return (format!("│ {}", inline(quote)), LineKind::Quote);
    }
    for bullet in ["- ", "* ", "+ "] {
        if let Some(text) = trimmed.strip_prefix(bullet) {
            let indent = &line[..line.len() - trimmed.len()];
            return (format!("{indent}• {}", inline(text)), LineKind::Text);
        }
    }
    (inline(line), LineKind::Text)
}
fn inline(line: &str) -> String {
    let mut result = String::new();
    let mut rest = line;
    while !rest.is_empty() {
        let mut consumed = false;
        for marker in ["`", "**", "__", "*", "_"] {
            if let Some(after) = rest.strip_prefix(marker) {
                if let Some(end) = after.find(marker).filter(|end| *end > 0) {
                    // Underscores inside identifiers remain literal.
                    if marker.contains('_')
                        && result.chars().last().is_some_and(|c| c.is_alphanumeric())
                    {
                        continue;
                    }
                    result.push_str(&after[..end]);
                    rest = &after[end + marker.len()..];
                    consumed = true;
                    break;
                }
            }
        }
        if consumed {
            continue;
        }
        if let Some(after) = rest.strip_prefix('[') {
            if let Some(close) = after.find("](") {
                if let Some(end) = after[close + 2..].find(')') {
                    let label = &after[..close];
                    let target = &after[close + 2..close + 2 + end];
                    result.push_str(&format!("{label} ({target})"));
                    rest = &after[close + 3 + end..];
                    continue;
                }
            }
            // An unmatched opener stays literal. Consuming the tail also avoids
            // repeatedly scanning a large malformed line for a nonexistent close.
            result.push_str(rest);
            break;
        }
        let c = rest.chars().next().unwrap();
        result.push(c);
        rest = &rest[c.len_utf8()..];
    }
    result
}
fn wrap(line: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut rest = line;
    while !rest.is_empty() {
        let mut used = 0;
        let mut end = 0;
        let mut space = None;
        for (index, c) in rest.char_indices() {
            let size = measure_text_width(&c.to_string());
            if used + size > width {
                break;
            }
            used += size;
            end = index + c.len_utf8();
            if c == ' ' && index > 0 {
                space = Some(index);
            }
        }
        if end == rest.len() {
            break;
        }
        if end == 0 {
            // A wide glyph cannot fit a one-column viewport.
            let c = rest.chars().next().unwrap();
            rows.push("�".into());
            rest = &rest[c.len_utf8()..];
            continue;
        }
        let split = space.unwrap_or(end);
        rows.push(rest[..split].to_owned());
        rest = rest[split..].trim_start_matches(' ');
    }
    rows.push(rest.to_owned());
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn markdown_keeps_source_mapping_and_code_literal_and_neutralizes_controls() {
        let block = Block { source: "test".into(), status: "ok".into(), format: ContentFormat::Markdown,
            content: "# Heading\n- **bold** and [link](https://example.com)\n```rust\nlet x = \"**literal**\";\n```\n> quote\n界界界\n\x1b[2J".into() };
        let rows = layout(&block, false, 18);
        assert_eq!(rows[0].text, "Heading");
        assert_eq!(rows[0].kind, LineKind::Heading);
        assert!(rows.iter().filter(|row| row.source == 1).count() > 1);
        assert!(rows
            .iter()
            .any(|row| row.source == 3 && row.text.contains("**literal**")));
        assert!(rows.iter().all(|row| measure_text_width(&row.text) <= 18));
        assert!(rows.iter().all(|row| !row.text.contains('\x1b')));
        assert_eq!(layout(&block, true, 80)[0].text, "# Heading");
    }
}
