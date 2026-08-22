//! Byte-offset <-> LSP `Position` conversion (UTF-16 code units, per the LSP
//! spec's default position encoding) and semantic-token delta encoding.

use tower_lsp::lsp_types::{Position, SemanticToken};

use crate::analysis::RawToken;

pub fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (i, b) in source.bytes().enumerate() {
        if b == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

pub fn offset_to_position(source: &str, line_starts: &[usize], offset: usize) -> Position {
    let line = match line_starts.binary_search(&offset) {
        Ok(l) => l,
        Err(l) => l.saturating_sub(1),
    };
    let line_start = line_starts[line];
    let character = source[line_start..offset].encode_utf16().count() as u32;
    Position {
        line: line as u32,
        character,
    }
}

pub fn position_to_offset(source: &str, position: Position) -> usize {
    let starts = line_starts(source);
    let Some(&line_start) = starts.get(position.line as usize) else {
        return source.len();
    };
    let line_end = starts
        .get(position.line as usize + 1)
        .copied()
        .unwrap_or(source.len());
    let line_text = &source[line_start..line_end];

    let mut utf16_count = 0u32;
    for (byte_idx, ch) in line_text.char_indices() {
        if utf16_count >= position.character {
            return line_start + byte_idx;
        }
        utf16_count += ch.len_utf16() as u32;
    }
    line_end
}

/// Sort raw byte-range tokens by position and delta-encode them into the
/// flat `SemanticToken` array the LSP wire format expects.
pub fn build_semantic_tokens(source: &str, raw: &[RawToken]) -> Vec<SemanticToken> {
    let starts = line_starts(source);
    let mut positioned: Vec<(Position, u32, u32)> = raw
        .iter()
        .map(|t| {
            let pos = offset_to_position(source, &starts, t.start);
            let len = source[t.start..t.end].encode_utf16().count() as u32;
            (pos, len, t.token_type)
        })
        .collect();
    positioned.sort_by_key(|(pos, _, _)| (pos.line, pos.character));

    let mut out = Vec::with_capacity(positioned.len());
    let mut prev_line = 0u32;
    let mut prev_char = 0u32;
    for (pos, length, token_type) in positioned {
        let delta_line = pos.line - prev_line;
        let delta_start = if delta_line == 0 {
            pos.character - prev_char
        } else {
            pos.character
        };
        out.push(SemanticToken {
            delta_line,
            delta_start,
            length,
            token_type,
            token_modifiers_bitset: 0,
        });
        prev_line = pos.line;
        prev_char = pos.character;
    }
    out
}
