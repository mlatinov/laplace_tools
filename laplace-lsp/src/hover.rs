//! Hover markdown, built on top of `laplace::docs`'s terminal rendering.
//!
//! `laplace::docs::render_overloads` produces one plain-text block meant for
//! a terminal: a signature line, then `Math:` / `Parameters:` / `Returns:` /
//! `Example:` sections. Wrapping the whole thing in one unlabelled code
//! fence is right for every section but `Math:`, whose body is LaTeX that a
//! client can render as a formula -- but only if it can find it.
//!
//! So the math sections are lifted out into their own ```` ```math ```` fences.
//! Nothing renders LaTeX in a VS Code hover natively; the fence is a marker
//! for the extension client, which substitutes a rendered image. A client
//! that does not (or a user who turned rendering off) still sees the LaTeX
//! as a code block, which is what the terminal showed anyway.
//!
//! Splitting is textual, which couples this module to that rendering. The
//! tests below therefore assert against output from `render_overloads`
//! itself rather than a hand-written copy of it, so a change to the
//! compiler's format fails here instead of silently emitting the math twice.

/// The section header `laplace::docs` writes above a `@math` body.
const MATH_HEADER: &str = "Math:";

/// The indent it writes on each line of that body.
const MATH_INDENT: &str = "  ";

/// Hover markdown for an already-rendered doc block: every section but the
/// math in one code fence, then one ```` ```math ```` fence per math section,
/// in the order they appeared.
pub fn markdown(rendered: &str) -> String {
    let (body, maths) = split_math_sections(rendered);

    let mut value = String::new();
    value.push_str("```\n");
    value.push_str(&body);
    if !body.ends_with('\n') {
        value.push('\n');
    }
    value.push_str("```\n");

    for math in &maths {
        value.push_str("\n```math\n");
        value.push_str(math);
        if !math.ends_with('\n') {
            value.push('\n');
        }
        value.push_str("```\n");
    }

    value
}

/// Remove every `Math:` section from `rendered`, returning what is left and
/// the dedented body of each section removed.
///
/// A section is the line `Math:` plus every following line indented by two
/// spaces -- the shape [`laplace::docs`] writes. The blank line that
/// separates it from whatever came before goes with it, so removing a
/// section never leaves a double blank line behind.
fn split_math_sections(rendered: &str) -> (String, Vec<String>) {
    let mut kept: Vec<&str> = Vec::new();
    let mut maths: Vec<String> = Vec::new();

    let lines: Vec<&str> = rendered.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim_end() != MATH_HEADER {
            kept.push(lines[i]);
            i += 1;
            continue;
        }

        // Drop the blank separator line above the header along with it.
        if kept.last().is_some_and(|l| l.trim().is_empty()) {
            kept.pop();
        }

        i += 1; // past `Math:`
        let mut math = String::new();
        while i < lines.len() {
            let Some(rest) = lines[i].strip_prefix(MATH_INDENT) else {
                break;
            };
            math.push_str(rest);
            math.push('\n');
            i += 1;
        }
        // A `Math:` header with no indented body is not a math section.
        if math.is_empty() {
            kept.push(MATH_HEADER);
        } else {
            maths.push(math);
        }
    }

    let mut body = kept.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    (body, maths)
}

#[cfg(test)]
mod tests {
    use laplace::parser::signatures::{Doc, FunctionSig};

    use super::*;

    fn sig(math: Option<&str>) -> FunctionSig {
        FunctionSig {
            name: "rbf_cov".to_string(),
            params: vec![("x".to_string(), "vector".to_string())],
            return_type: "matrix".to_string(),
            doc: Some(Doc {
                brief: Some("Radial basis covariance.".to_string()),
                params: vec![("x".to_string(), "the inputs".to_string())],
                return_doc: Some("the covariance matrix".to_string()),
                example: Some("rbf_cov([1.0]')".to_string()),
                math: math.map(str::to_string),
            }),
        }
    }

    /// The coupling test: the split runs against real `render_overloads`
    /// output, so reformatting the compiler's doc rendering fails here.
    #[test]
    fn math_is_lifted_out_of_real_rendered_output() {
        let math = "\\pi_n = \\operatorname{logit}^{-1}(\\psi_n) \\\\\n\\log L = \\sum_n d_n";
        let rendered = laplace::docs::render_overloads("gps", &[sig(Some(math))]);
        let (body, maths) = split_math_sections(&rendered);

        assert_eq!(maths.len(), 1, "expected one math section in:\n{rendered}");
        assert_eq!(maths[0], format!("{math}\n"));
        assert!(!body.contains("Math:"), "header left behind in:\n{body}");
        assert!(!body.contains("\\operatorname"), "LaTeX left behind in:\n{body}");
        // Every other section survives untouched.
        assert!(body.contains("Radial basis covariance."));
        assert!(body.contains("Parameters:"));
        assert!(body.contains("the covariance matrix"));
        assert!(body.contains("rbf_cov([1.0]')"));
        assert!(!body.contains("\n\n\n"), "double blank line left behind in:\n{body}");
    }

    #[test]
    fn a_function_without_math_is_passed_through_unchanged() {
        let rendered = laplace::docs::render_overloads("gps", &[sig(None)]);
        let (body, maths) = split_math_sections(&rendered);
        assert!(maths.is_empty());
        assert_eq!(body, rendered);
    }

    #[test]
    fn each_overload_group_contributes_its_own_fence() {
        let mut second = sig(Some("y = x^2"));
        second.params = vec![("x".to_string(), "real".to_string())];
        second.doc.as_mut().unwrap().brief = Some("Scalar overload.".to_string());
        let rendered = laplace::docs::render_overloads("gps", &[sig(Some("K = x x^T")), second]);

        let value = markdown(&rendered);
        assert_eq!(value.matches("```math").count(), 2, "in:\n{value}");
        assert!(value.contains("K = x x^T"));
        assert!(value.contains("y = x^2"));
    }

    #[test]
    fn the_body_is_fenced_and_the_math_is_not_inside_it() {
        let rendered = laplace::docs::render_overloads("gps", &[sig(Some("E = mc^2"))]);
        let value = markdown(&rendered);
        let (before, after) = value.split_once("```math").expect("a math fence");
        assert!(!before.contains("E = mc^2"), "math inside the body fence:\n{before}");
        assert!(after.contains("E = mc^2"));
    }

    #[test]
    fn a_bare_math_header_with_no_body_is_left_alone() {
        let (body, maths) = split_math_sections("sig\n\nMath:\nParameters:\n  x  the inputs\n");
        assert!(maths.is_empty());
        assert!(body.contains("Math:"));
    }
}
