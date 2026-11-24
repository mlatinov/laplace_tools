//! Where the cursor is in a `.lapkernel` or `.lapsampler`, read from the
//! text alone. The text is usually mid-edit and does not parse, so this is
//! lexical: comments are blanked out, then braces, parentheses and commas
//! are counted. Offsets are bytes into the document; every character this
//! looks for is ASCII.

use std::ops::Range;

/// `text` with every comment character replaced by spaces (one per byte,
/// newlines kept), so offsets are unchanged and nothing in a comment is
/// mistaken for code.
pub fn mask_comments(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                out[i] = b' ';
                i += 1;
            }
        } else if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            let end = text[i + 2..].find("*/").map_or(b.len(), |e| i + 2 + e + 2);
            for o in &mut out[i..end] {
                if *o != b'\n' {
                    *o = b' ';
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }
    // Only whole characters were replaced, by ASCII spaces.
    String::from_utf8(out).expect("comments are blanked whole")
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// A `:` that is not half of `::`.
fn is_single_colon(b: &[u8], i: usize) -> bool {
    b[i] == b':' && b.get(i + 1) != Some(&b':') && (i == 0 || b[i - 1] != b':')
}

/// What ends a sampler token: whitespace, `( ) , ; = { }`, a single `:`.
fn ends_token(b: &[u8], i: usize) -> bool {
    b[i].is_ascii_whitespace() || b"(),;={}".contains(&b[i]) || is_single_colon(b, i)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TokenKind {
    /// `mala`, `langevin::mala`, `@adapt`.
    Name,
    /// `./kernels/k.lapkernel`, `../k.lapkernel`, `/abs/k.lapkernel`.
    Path,
    /// Anything else (a number, an operator).
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub range: Range<usize>,
    pub text: String,
}

fn classify(text: &str) -> TokenKind {
    let b = text.as_bytes();
    if text.starts_with("./") || text.starts_with("../") || text.starts_with('/') {
        return TokenKind::Path;
    }
    let ident_run = |mut i: usize| {
        if !b
            .get(i)
            .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_')
        {
            return None;
        }
        while b.get(i).copied().is_some_and(is_ident) {
            i += 1;
        }
        Some(i)
    };
    let start = usize::from(b.first() == Some(&b'@'));
    let mut i = match ident_run(start) {
        Some(i) => i,
        None => return TokenKind::Other,
    };
    while text[i..].starts_with("::") {
        match ident_run(i + 2) {
            Some(j) => i = j,
            None => return TokenKind::Other,
        }
    }
    if i == b.len() {
        TokenKind::Name
    } else {
        TokenKind::Other
    }
}

/// The token around `offset` (the cursor may be inside it or just after
/// it).
pub fn token_at(masked: &str, offset: usize) -> Option<Token> {
    let b = masked.as_bytes();
    let offset = offset.min(b.len());
    let mut start = offset;
    while start > 0 && !ends_token(b, start - 1) {
        start -= 1;
    }
    let mut end = offset;
    while end < b.len() && !ends_token(b, end) {
        end += 1;
    }
    (start < end).then(|| Token {
        kind: classify(&masked[start..end]),
        range: start..end,
        text: masked[start..end].to_string(),
    })
}

/// The innermost call whose `(` is open at `offset`, looking back no
/// further than `from`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    /// The token before the `(`: `mala`, `langevin::mala`, a path, `seq`.
    pub callee: Token,
    /// Offset of the `(`.
    pub open: usize,
    /// Which argument the cursor is in (0-based, by top-level commas).
    pub arg: usize,
    /// Where that argument starts.
    pub arg_start: usize,
}

pub fn enclosing_call(masked: &str, from: usize, offset: usize) -> Option<Call> {
    let b = masked.as_bytes();
    let offset = offset.min(b.len());
    let mut depth = 0usize;
    let mut commas = vec![];
    let mut i = offset;
    while i > from {
        i -= 1;
        match b[i] {
            b')' => depth += 1,
            b'(' if depth > 0 => depth -= 1,
            b'(' => {
                let mut end = i;
                while end > from && b[end - 1].is_ascii_whitespace() {
                    end -= 1;
                }
                let callee = token_at(masked, end).filter(|t| t.range.end == end)?;
                return Some(Call {
                    callee,
                    open: i,
                    arg: commas.len(),
                    arg_start: commas.first().map_or(i + 1, |c| c + 1),
                });
            }
            b',' if depth == 0 => commas.push(i),
            b'{' | b'}' | b';' => return None,
            _ => {}
        }
    }
    None
}

/// Where the `.lapsampler` kernel line around `offset` starts: just after
/// the `{` of `kernel {`, a `;`, or a newline that is not inside an open
/// parenthesis.
pub fn kernel_line_start(masked: &str, offset: usize) -> usize {
    let b = masked.as_bytes();
    let offset = offset.min(b.len());
    // Forward from the last `{`: a line ends at `;` or at a newline with
    // no parenthesis open, so a call may span lines.
    let mut start = masked[..offset].rfind('{').map_or(0, |i| i + 1);
    let mut depth = 0i64;
    for (i, c) in b.iter().enumerate().take(offset).skip(start) {
        match c {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b';' => {
                depth = 0;
                start = i + 1;
            }
            b'\n' if depth <= 0 => {
                depth = 0;
                start = i + 1;
            }
            _ => {}
        }
    }
    start
}

/// The next character after `offset` that is not whitespace.
pub fn next_char(masked: &str, offset: usize) -> Option<char> {
    masked[offset.min(masked.len())..]
        .chars()
        .find(|c| !c.is_whitespace())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `text` with `|` marking the cursor: (text, offset).
    fn at(marked: &str) -> (String, usize) {
        let offset = marked.find('|').unwrap();
        (marked.replacen('|', "", 1), offset)
    }

    #[test]
    fn comments_are_blanked_keeping_offsets() {
        let text = "a // 𝜎 (x\nb /* { 𝜎\n } */ c";
        let m = mask_comments(text);
        assert_eq!(m.len(), text.len());
        assert_eq!(m.matches('\n').count(), 2);
        assert!(!m.contains('{') && !m.contains('(') && !m.contains('𝜎'));
        assert!(m.starts_with("a ") && m.ends_with(" c"));
    }

    #[test]
    fn tokens() {
        let t = |marked: &str| {
            let (s, o) = at(marked);
            token_at(&mask_comments(&s), o).map(|t| (t.kind, t.text))
        };
        assert_eq!(
            t("rest : lang|evin::mala(eps = 1)"),
            Some((TokenKind::Name, "langevin::mala".into()))
        );
        assert_eq!(
            t("rest : ./k/m|ine.lapkernel(eps = 1)"),
            Some((TokenKind::Path, "./k/mine.lapkernel".into()))
        );
        assert_eq!(
            t("rest : ../m.lapkernel|"),
            Some((TokenKind::Path, "../m.lapkernel".into()))
        );
        assert_eq!(t("mix(1|:rwm())"), Some((TokenKind::Other, "1".into())));
        assert_eq!(t("mix(1:rw|m())"), Some((TokenKind::Name, "rwm".into())));
        assert_eq!(
            t("x = 1 @ad|apt(metric)"),
            Some((TokenKind::Name, "@adapt".into()))
        );
        assert_eq!(t("rest : | nuts()"), None);
        assert_eq!(
            t("rest : langevin::|"),
            Some((TokenKind::Other, "langevin::".into()))
        );
    }

    #[test]
    fn the_enclosing_call_and_argument() {
        let call = |marked: &str| {
            let (s, o) = at(marked);
            let m = mask_comments(&s);
            enclosing_call(&m, 0, o).map(|c| (c.callee.text, c.arg, m[c.arg_start..o].to_string()))
        };
        assert_eq!(call("seq(rwm(), |"), Some(("seq".into(), 1, " ".into())));
        assert_eq!(
            call("seq(rwm(step_size = 1, |"),
            Some(("rwm".into(), 1, " ".into()))
        );
        assert_eq!(
            call("adapt(langevin::mala(eps = 1), target|"),
            Some(("adapt".into(), 1, " target".into()))
        );
        assert_eq!(
            call("./k.lapkernel (e|"),
            Some(("./k.lapkernel".into(), 0, "e".into()))
        );
        assert_eq!(call("seq(rwm(), mala())|"), None);
        assert_eq!(call("step {\n  x = f(|"), Some(("f".into(), 0, "".into())));
        assert_eq!(call("f(a);\n  x = |"), None);
    }

    #[test]
    fn kernel_lines_start_after_a_newline_outside_parentheses() {
        let (s, o) = at("  kernel {\n    block(a) : seq(rwm(),\n      mala(|");
        let m = mask_comments(&s);
        let start = kernel_line_start(&m, o);
        assert_eq!(&m[start..start + 9], "    block");
        let (s, o) = at("  kernel {\n    block(a) : rwm(); rest : |");
        let m = mask_comments(&s);
        assert_eq!(&m[kernel_line_start(&m, o)..o], " rest : ");
    }
}
