//! Each case is a small kernel or sampler broken in one way, with the exact
//! LSP diagnostics it must get: range (0-based, UTF-16) and message. The
//! messages are the checker's own (`laplace_sampler_syntax::editor`); what
//! is tested here is where they land.

use super::*;

/// A kernel that checks. The cases below each break it in one place.
const KERNEL: &str = "kernel k {
  tuning {
    real<lower=0> eps = 1 @adapt(step_size, target_accept = 0.574);
  }
  step {
    vector[dim] q_new = q + eps * std_normal_rng(dim);
    propose(q_new, 0);
  }
}
";

/// `KERNEL` with `from` replaced by `to`.
fn kernel(from: &str, to: &str) -> String {
    assert!(KERNEL.contains(from), "{from}");
    KERNEL.replacen(from, to, 1)
}

/// (start line, start character, end line, end character, message).
type Expected<'a> = (u32, u32, u32, u32, &'a str);

fn found(ds: &[Diagnostic]) -> Vec<(u32, u32, u32, u32, String)> {
    ds.iter()
        .map(|d| {
            let r = d.range;
            (
                r.start.line,
                r.start.character,
                r.end.line,
                r.end.character,
                d.message.clone(),
            )
        })
        .collect()
}

fn assert_diagnostics(ds: &[Diagnostic], expected: &[Expected]) {
    let want: Vec<(u32, u32, u32, u32, String)> = expected
        .iter()
        .map(|&(a, b, c, d, m)| (a, b, c, d, m.to_string()))
        .collect();
    assert_eq!(found(ds), want);
    for d in ds {
        assert_eq!(d.severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(d.source.as_deref(), Some(SOURCE));
    }
}

fn check_kernel(src: &str) -> Vec<Diagnostic> {
    diagnostics(Language::Kernel, Path::new("/p/k.lapkernel"), src)
}

// ---- .lapkernel ---------------------------------------------------------------

#[test]
fn a_kernel_that_checks_has_no_diagnostics() {
    assert_diagnostics(&check_kernel(KERNEL), &[]);
}

#[test]
fn the_shipped_kernels_have_no_diagnostics() {
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../vscode-laplace/fixtures/sampler"
    );
    for name in [
        "laplace-sample/mala.lapkernel",
        "laplace-sample/rwm.lapkernel",
        "laplace-langevin/mala.lapkernel",
        "laplace-langevin/barker.lapkernel",
    ] {
        let path = Path::new(dir).join(name);
        let src = std::fs::read_to_string(&path).unwrap();
        assert_diagnostics(&diagnostics(Language::Kernel, &path, &src), &[]);
    }
}

#[test]
fn a_syntax_error_marks_the_token_it_stopped_at() {
    assert_diagnostics(
        &check_kernel(&kernel("propose(q_new, 0);", "propose(q_new, 0)")),
        &[(
            7,
            2,
            7,
            3,
            "syntax error: expected an operator, `[` or `.N`",
        )],
    );
}

#[test]
fn a_syntax_error_at_the_end_of_the_file_is_at_its_end() {
    assert_diagnostics(
        &check_kernel("kernel k {\n  step {\n"),
        &[(2, 0, 2, 0, "syntax error: expected a statement")],
    );
}

#[test]
fn unknown_names_and_functions_mark_the_name() {
    assert_diagnostics(
        &check_kernel(&kernel("q + eps", "q + nope")),
        &[(5, 28, 5, 32, "unknown name `nope`")],
    );
    assert_diagnostics(
        &check_kernel(&kernel("std_normal_rng(dim)", "foo(dim)")),
        &[(5, 34, 5, 37, "unknown function `foo`")],
    );
}

#[test]
fn a_kernel_without_a_step_block() {
    assert_diagnostics(
        &check_kernel("kernel k {\n  tuning {\n    real eps = 1;\n  }\n}\n"),
        &[(0, 0, 0, 6, "kernel `k` has no `step { }` block")],
    );
}

#[test]
fn adapt_tags() {
    assert_diagnostics(
        &check_kernel(&kernel(
            "@adapt(step_size, target_accept = 0.574)",
            "@adapt(stepsize)",
        )),
        &[(
            2,
            26,
            2,
            32,
            "unknown @adapt tag `stepsize`: expected step_size or metric",
        )],
    );
    assert_diagnostics(
        &check_kernel(&kernel("0.574", "1.5")),
        &[(
            2,
            60,
            2,
            63,
            "target_accept must be a number strictly between 0 and 1",
        )],
    );
}

#[test]
fn type_errors_mark_each_place() {
    assert_diagnostics(
        &check_kernel(&kernel("vector[dim] q_new = q", "int q_new = q")),
        &[
            (5, 16, 5, 17, "`q_new` is int, but this is vector"),
            (
                6,
                4,
                6,
                11,
                "propose: expected propose(vector[dim] q_new, real log_correction), got (int, int)",
            ),
        ],
    );
    assert_diagnostics(
        &check_kernel(&kernel(
            "eps * std_normal_rng(dim)",
            "q * std_normal_rng(dim)",
        )),
        &[(
            5,
            28,
            5,
            29,
            "vector * vector is not defined; use .* (elementwise) or dot_product(a, b)",
        )],
    );
    assert_diagnostics(
        &check_kernel(&kernel(
            "propose(q_new, 0);",
            "propose(q_new, normal_lpdf(q_new, 0, 1));",
        )),
        &[(
            6,
            19,
            6,
            30,
            "write normal_lpdf(y | ...): the outcome is separated by `|`, as in Stan",
        )],
    );
    assert_diagnostics(
        &check_kernel(&kernel(
            "    propose(q_new, 0);\n",
            "    sum(q_new);\n    propose(q_new, 0);\n",
        )),
        &[(6, 4, 6, 7, "the real returned by `sum(...)` is not used")],
    );
}

#[test]
fn every_step_ends_in_exactly_one_terminal() {
    const NEVER: &str = "step { } never calls propose()/move(): every step must end in exactly one";
    assert_diagnostics(
        &check_kernel(&kernel("    propose(q_new, 0);\n", "")),
        &[(4, 7, 4, 8, NEVER)],
    );
    assert_diagnostics(
        &check_kernel(&kernel("    propose(q_new, 0);\n", "    return 1;\n")),
        &[
            (4, 7, 4, 8, NEVER),
            (
                6,
                4,
                6,
                10,
                "`return` is only allowed in functions; a step ends with propose()/move()",
            ),
        ],
    );
    assert_diagnostics(
        &check_kernel(&kernel(
            "    propose(q_new, 0);\n",
            "    for (i in 1:2) {\n      propose(q_new, 0);\n    }\n",
        )),
        &[
            (4, 7, 4, 8, NEVER),
            (
                7,
                6,
                7,
                13,
                "propose()/move() inside a loop: a step must end in exactly one, on every path",
            ),
        ],
    );
}

#[test]
fn built_ins_cannot_be_assigned_or_redeclared() {
    assert_diagnostics(
        &check_kernel(&kernel(
            "    propose(q_new, 0);\n",
            "    q = q_new;\n    propose(q_new, 0);\n",
        )),
        &[(6, 4, 6, 5, "`q` is a built-in and cannot be assigned")],
    );
    assert_diagnostics(
        &check_kernel(&kernel("vector[dim] q_new = q +", "vector[dim] grad = q +")),
        &[
            (5, 4, 5, 10, "`grad` is a built-in and cannot be redeclared"),
            (6, 12, 6, 17, "unknown name `q_new`"),
        ],
    );
}

#[test]
fn columns_after_a_character_outside_the_bmp_count_utf16_units() {
    // `𝜎` is one character to the checker (column 37, 1-based) and two
    // UTF-16 code units to the editor.
    let src = kernel("q + eps", "q + /* 𝜎 */ nope");
    let ds = check_kernel(&src);
    assert_diagnostics(&ds, &[(5, 37, 5, 41, "unknown name `nope`")]);
    let line = src.lines().nth(5).unwrap();
    let units: Vec<u16> = line.encode_utf16().collect();
    assert_eq!(String::from_utf16(&units[37..41]).unwrap(), "nope");
}

// ---- .lapsampler --------------------------------------------------------------

/// A directory with a kernel that checks, one with nothing to adapt and one
/// with two errors.
fn kernels_dir() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    std::fs::write(dir.join("good.lapkernel"), KERNEL).unwrap();
    std::fs::write(
        dir.join("plain.lapkernel"),
        KERNEL.replace(" @adapt(step_size, target_accept = 0.574)", ""),
    )
    .unwrap();
    std::fs::write(
        dir.join("broken.lapkernel"),
        kernel("q + eps", "q + nope").replace(
            "    propose(q_new, 0);",
            "    sum(q_new);\n    propose(q_new, 0);",
        ),
    )
    .unwrap();
    tmp
}

fn sampler(kernel_line: &str) -> String {
    format!("sampler s {{\n  kernel {{\n    {kernel_line}\n  }}\n}}\n")
}

fn check_sampler(dir: &Path, src: &str) -> Vec<Diagnostic> {
    diagnostics(Language::Sampler, &dir.join("s.lapsampler"), src)
}

#[test]
fn a_sampler_that_checks_has_no_diagnostics() {
    let tmp = kernels_dir();
    for line in [
        "rest : nuts()",
        "rest : adapt(./good.lapkernel)",
        "rest : seq(mala(step_size = 1e-2), rwm())",
    ] {
        assert_diagnostics(&check_sampler(tmp.path(), &sampler(line)), &[]);
    }
}

#[test]
fn built_in_kernels_and_their_settings() {
    let tmp = kernels_dir();
    assert_diagnostics(
        &check_sampler(tmp.path(), &sampler("rest : nutz()")),
        &[(
            2,
            11,
            2,
            15,
            "unknown kernel `nutz`: the built-in kernels are nuts, rwm, mala and hmc; a local kernel is a path such as ./kernels/nutz.lapkernel; a library kernel is <package>::nutz",
        )],
    );
    assert_diagnostics(
        &check_sampler(tmp.path(), &sampler("rest : nuts(step_sise = 0.1)")),
        &[(
            2,
            16,
            2,
            25,
            "nuts() has no setting `step_sise`; its settings are: step_size, target_accept, max_depth",
        )],
    );
    assert_diagnostics(
        &check_sampler(tmp.path(), &sampler("rest : nuts(max_depth = 0.5)")),
        &[(
            2,
            11,
            2,
            15,
            "nuts(): max_depth: `0.5` is not a whole number",
        )],
    );
}

#[test]
fn a_syntax_error() {
    let tmp = kernels_dir();
    assert_diagnostics(
        &check_sampler(tmp.path(), &sampler("rest nuts()")),
        &[(
            2,
            4,
            2,
            8,
            "syntax error: expected a kernel line (`block(...) : kernel` or `rest : kernel`)",
        )],
    );
}

#[test]
fn a_missing_local_kernel_marks_its_path() {
    let tmp = kernels_dir();
    let ds = check_sampler(tmp.path(), &sampler("rest : ./nope.lapkernel"));
    assert_eq!(ds.len(), 1, "{ds:#?}");
    let r = ds[0].range;
    assert_eq!(
        (r.start.line, r.start.character, r.end.character),
        (2, 11, 27)
    );
    let path = tmp.path().join("./nope.lapkernel");
    assert!(
        ds[0]
            .message
            .starts_with(&format!("cannot read {}: ", path.display())),
        "{}",
        ds[0].message
    );
}

#[test]
fn a_local_kernel_that_does_not_check_links_to_each_of_its_errors() {
    let tmp = kernels_dir();
    // The checker names it as the sampler's directory joined with the path
    // as written.
    let broken = tmp.path().join("./broken.lapkernel");
    let ds = check_sampler(tmp.path(), &sampler("rest : ./broken.lapkernel"));
    let message = format!(
        "./broken.lapkernel does not check:\n  {b}:6:29: unknown name `nope`\n  {b}:7:5: the real returned by `sum(...)` is not used",
        b = broken.display()
    );
    assert_diagnostics(&ds, &[(2, 11, 2, 29, &message)]);

    let related = ds[0].related_information.clone().unwrap();
    let uri = Url::from_file_path(&broken).unwrap();
    let got: Vec<(Url, Range, String)> = related
        .into_iter()
        .map(|r| (r.location.uri, r.location.range, r.message))
        .collect();
    assert_eq!(
        got,
        vec![
            (
                uri.clone(),
                Range::new(Position::new(5, 28), Position::new(5, 32)),
                "unknown name `nope`".to_string()
            ),
            (
                uri,
                Range::new(Position::new(6, 4), Position::new(6, 7)),
                "the real returned by `sum(...)` is not used".to_string()
            ),
        ]
    );
}

#[test]
fn adapt_needs_something_to_adapt() {
    let tmp = kernels_dir();
    assert_diagnostics(
        &check_sampler(tmp.path(), &sampler("rest : adapt(./plain.lapkernel)")),
        &[(
            2,
            11,
            2,
            16,
            "adapt(): ./plain.lapkernel has nothing to adapt: tag its step size @adapt(step_size, target_accept = ...) and/or its inverse metric @adapt(metric)",
        )],
    );
}

#[test]
fn an_import_outside_a_project_says_there_is_no_lockfile() {
    let tmp = kernels_dir();
    let src = "sampler s {\n  library { import langevin }\n  kernel {\n    rest : langevin::mala\n  }\n}\n";
    let message = format!(
        "`import langevin`: there is no laplace.lock in {} -- run from the project directory; add it with `laplace add langevin --path <dir>` (or `--git <url> --tag <tag>`)",
        tmp.path().display()
    );
    assert_diagnostics(&check_sampler(tmp.path(), src), &[(1, 12, 1, 18, &message)]);
}

#[test]
fn the_project_is_the_nearest_directory_with_a_lockfile() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let nested = root.join("samplers/deep");
    std::fs::create_dir_all(&nested).unwrap();
    assert_eq!(project_for(&nested).dir, nested);
    laplace::resolve::lockfile::write_lockfile(
        &root.join("laplace.lock"),
        &laplace::resolve::lockfile::Lockfile::default(),
    )
    .unwrap();
    assert_eq!(project_for(&nested).dir, root);

    // So an import is looked up in that lockfile, and is not in it.
    let src = "sampler s {\n  library { import langevin }\n  kernel {\n    rest : langevin::mala\n  }\n}\n";
    let ds = check_sampler(&nested, src);
    assert_eq!(ds.len(), 1, "{ds:#?}");
    assert!(
        !ds[0].message.contains("there is no laplace.lock"),
        "{}",
        ds[0].message
    );
}

#[test]
fn sampler_columns_count_utf16_units_too() {
    let tmp = kernels_dir();
    let src = "// 𝜎\nsampler s {\n  kernel {\n    rest : /*𝜎𝜎*/ nutz()\n  }\n}\n";
    let ds = check_sampler(tmp.path(), src);
    assert_eq!(ds.len(), 1, "{ds:#?}");
    let r = ds[0].range;
    assert_eq!(
        (r.start.line, r.start.character, r.end.character),
        (3, 20, 24)
    );
}

// ---- routing and ranges ------------------------------------------------------

#[test]
fn only_the_two_sampler_extensions_are_routed_here() {
    let lang = |p: &str| Language::for_path(Path::new(p));
    assert_eq!(lang("/m/k.lapkernel"), Some(Language::Kernel));
    assert_eq!(lang("/m/s.lapsampler"), Some(Language::Sampler));
    assert_eq!(lang("/m/model.laplace"), None);
    assert_eq!(lang("/m/stats.laplacelib"), None);
    assert_eq!(lang("/m/README"), None);
    let untitled = Url::parse("untitled:Untitled-1").unwrap();
    assert_eq!(Language::for_uri(&untitled), None);
}

#[test]
fn token_ranges() {
    let r = |src: &str, line, col| {
        let r = token_range(src, line, col);
        (r.start.line, r.start.character, r.end.character)
    };
    let src = "a : langevin::mala(eps = -1.5E+1, x = .5)\nb : ../k/x.lapkernel(1)\r\nend";
    assert_eq!(r(src, 1, 5), (0, 4, 18), "pkg::name");
    assert_eq!(r(src, 1, 26), (0, 25, 32), "signed exponent");
    assert_eq!(r(src, 1, 39), (0, 38, 40), ".5");
    assert_eq!(r(src, 1, 19), (0, 18, 19), "punctuation is one character");
    assert_eq!(r(src, 2, 5), (1, 4, 20), "a path, up to `(`");
    assert_eq!(r(src, 2, 99), (1, 22, 23), "past the end of a CRLF line");
    assert_eq!(r(src, 9, 1), (2, 2, 3), "past the last line");
    assert_eq!(r("@adapt(m)", 1, 1), (0, 0, 6), "@adapt");
    assert_eq!(r("x // c", 1, 3), (0, 2, 3), "a comment is not a path");
    assert_eq!(token_range(src, 0, 0), Range::default(), "the whole file");
}

#[test]
fn a_diagnostic_in_another_file_stays_at_the_top_with_its_location() {
    let d = editor::Diagnostic {
        file: "other.lapkernel".into(),
        line: 3,
        column: 4,
        severity: Severity::Error,
        message: "oops".into(),
    };
    let lsp = to_lsp("x\n", "/p/s.lapsampler", &d);
    assert_eq!(lsp.range, Range::default());
    assert_eq!(lsp.message, "other.lapkernel:3:4: oops");
}
