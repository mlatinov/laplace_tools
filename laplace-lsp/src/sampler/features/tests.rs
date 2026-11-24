//! Each test puts the cursor (`|`) somewhere in a kernel or sampler and
//! checks what is offered there. The sampler tests run in a temporary
//! project with a kernel package `langevin` (a path dependency: the
//! fixture mala and barker, plus a broken kernel) and a local kernel; the
//! package cache is temporary too, so nothing touches `~/.laplace`.

use laplace::manifest::PackageKind;
use laplace::resolve::lockfile::{write_lockfile, LockedPackage, Lockfile};

use super::*;

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../vscode-laplace/fixtures/sampler"
);

/// `text` with `|` marking the cursor: (text, offset).
pub fn at(marked: &str) -> (String, usize) {
    let offset = marked.find('|').expect("a | marks the cursor");
    (marked.replacen('|', "", 1), offset)
}

pub struct TestProject {
    _tmp: tempfile::TempDir,
    pub dir: PathBuf,
    pub index: Index,
}

pub fn project() -> TestProject {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let pkg = root.join("langevin");
    std::fs::create_dir_all(pkg.join("kernels")).unwrap();
    std::fs::write(
        pkg.join("laplace.toml"),
        "name = \"langevin\"\nversion = \"0.1.0\"\nkind = \"kernels\"\nlaplace = \">=0.3\"\n",
    )
    .unwrap();
    for k in ["mala", "barker"] {
        std::fs::copy(
            format!("{FIXTURES}/laplace-langevin/{k}.lapkernel"),
            pkg.join(format!("kernels/{k}.lapkernel")),
        )
        .unwrap();
    }
    std::fs::write(
        pkg.join("kernels/broken.lapkernel"),
        "kernel broken {\n  step {\n    move(nope);\n  }\n}\n",
    )
    .unwrap();
    let dir = root.join("project");
    std::fs::create_dir_all(dir.join("kernels")).unwrap();
    std::fs::copy(
        format!("{FIXTURES}/laplace-sample/mala.lapkernel"),
        dir.join("kernels/mine.lapkernel"),
    )
    .unwrap();
    let lock = Lockfile {
        root: vec!["langevin".into()],
        packages: vec![LockedPackage {
            kind: PackageKind::Kernels,
            ..LockedPackage::leaf("langevin", "0.1.0", "sha256:0", "path+../langevin")
        }],
    };
    write_lockfile(&dir.join("laplace.lock"), &lock).unwrap();
    let project = Project {
        dir: dir.clone(),
        cache_root: root.join("cache"),
    };
    let index = Index::load(&dir, &project);
    TestProject {
        _tmp: tmp,
        dir,
        index,
    }
}

const SAMPLER: &str = "sampler s {
  library { import langevin }
  kernel {
    block(a) : adapt(langevin::mala(eps = 0.1), target_accept = 0.6)
    block(b) : seq(rwm(step_size = 0.5), ./kernels/mine.lapkernel(eps = 1))
    block(c) : langevin::broken
    rest : nuts()
  }
}
";

/// SAMPLER with the cursor placed just after the first `find`.
fn in_sampler(find: &str, back: usize) -> (String, usize) {
    let i = SAMPLER.find(find).unwrap_or_else(|| panic!("{find}")) + find.len() - back;
    (SAMPLER.to_string(), i)
}

// ---- hover: .lapkernel ---------------------------------------------------------

#[test]
fn kernel_hover_shows_a_built_ins_signatures_doc_and_random_numbers() {
    let (src, o) = at("kernel k {\n  step {\n    vector[dim] z = std_nor|mal_rng(dim);\n    move(q + z);\n  }\n}\n");
    let (range, md) = hover(Language::Kernel, &src, o, None).unwrap();
    assert_eq!(&src[range], "std_normal_rng");
    assert_eq!(
        md,
        "```lapkernel\nreal std_normal_rng()\nvector[n] std_normal_rng(int n)\n```\n\n\
         independent N(0, 1) draws: one real, or a vector of n\n\n\
         *Random numbers:* one normal draw (rand_distr::StandardNormal) per element, in index order"
    );
}

#[test]
fn kernel_hover_on_context_values_and_terminals() {
    let src = "kernel k {\n  step {\n    move(q + dim);\n  }\n}\n";
    let md = |word: &str| {
        let o = src.find(word).unwrap() + 1;
        hover(Language::Kernel, src, o, None).map(|(_, md)| md)
    };
    assert!(md("q +")
        .unwrap()
        .starts_with("```lapkernel\nvector[dim] q\n```\n\ncurrent position"));
    let dim = md("dim)").unwrap();
    assert!(dim.starts_with("```lapkernel\nint dim\n```"), "{dim}");
    assert!(dim.ends_with("*Also allowed in tuning defaults.*"), "{dim}");
    let mv = md("move").unwrap();
    assert!(
        mv.contains("void move(vector[dim] q_new)") && mv.contains("ends the step"),
        "{mv}"
    );
}

#[test]
fn kernel_hover_ignores_comments_locals_and_keywords() {
    let src =
        "kernel k {\n  // q is the position\n  step {\n    real x = 1;\n    move(q);\n  }\n}\n";
    for word in ["q is", "x =", "step", "real"] {
        let o = src.find(word).unwrap();
        assert_eq!(hover(Language::Kernel, src, o, None), None, "{word}");
    }
}

// ---- hover: .lapsampler --------------------------------------------------------

#[test]
fn sampler_hover_on_kernels_shows_their_documentation() {
    let p = project();
    assert_eq!(p.index.dir, p.dir);
    let doc = |name: &str| {
        p.index
            .kernels
            .iter()
            .find(|k| k.name == name)
            .unwrap()
            .doc
            .clone()
    };
    for (find, name) in [
        ("langevin::ma", "langevin::mala"),
        ("rw", "rwm"),
        ("nu", "nuts"),
        ("./kernels/mi", "./kernels/mine.lapkernel"),
    ] {
        let (src, o) = in_sampler(find, 0);
        let (range, md) = hover(Language::Sampler, &src, o, Some(&p.index)).unwrap();
        assert_eq!(&src[range.clone()], name);
        assert_eq!(
            md,
            format!("```text\n{}\n```", doc(name).trim_end()),
            "{name}"
        );
    }
    let (src, o) = in_sampler("langevin::ma", 0);
    let md = hover(Language::Sampler, &src, o, Some(&p.index)).unwrap().1;
    assert!(
        md.contains("eps = 1   @adapt(step_size, target_accept = 0.574)"),
        "{md}"
    );
    assert!(md.contains("ends in propose()"), "{md}");
}

#[test]
fn sampler_hover_on_a_kernel_that_does_not_check() {
    let p = project();
    let (src, o) = in_sampler("langevin::bro", 0);
    let md = hover(Language::Sampler, &src, o, Some(&p.index)).unwrap().1;
    assert!(
        md.starts_with("**langevin::broken**, kernel package `langevin` 0.1.0: does not check"),
        "{md}"
    );
    assert!(md.contains("unknown name `nope`"), "{md}");
}

#[test]
fn sampler_hover_on_combinators() {
    let p = project();
    let (src, o) = in_sampler("ada", 0);
    let md = hover(Language::Sampler, &src, o, Some(&p.index)).unwrap().1;
    assert!(
        md.starts_with("```lapsampler\nadapt(k, target_accept = ...)\n```\n\nadapt k's step size"),
        "{md}"
    );
    let (src, o) = in_sampler("se", 0);
    let md = hover(Language::Sampler, &src, o, Some(&p.index)).unwrap().1;
    assert!(
        md.starts_with("```lapsampler\nseq(k1, k2, ...)\n```"),
        "{md}"
    );
}

#[test]
fn sampler_hover_on_settings() {
    let p = project();
    let (src, o) = in_sampler("mala(ep", 0);
    let md = hover(Language::Sampler, &src, o, Some(&p.index)).unwrap().1;
    assert_eq!(
        md,
        "```lapkernel\nreal<lower=0> eps = 1\n```\n\n`@adapt(step_size, target_accept = 0.574)`\n\n\
         Step size (adapted toward 0.574 acceptance)\n\n\
         A setting of `langevin::mala` (kernel package `langevin` 0.1.0)."
    );
    let (src, o) = in_sampler("rwm(step", 0);
    let md = hover(Language::Sampler, &src, o, Some(&p.index)).unwrap().1;
    assert_eq!(
        md,
        "```lapkernel\nstep_size = 1\n```\n\nA setting of `rwm` (built-in kernel)."
    );
    let (src, o) = in_sampler("target_acc", 0);
    let md = hover(Language::Sampler, &src, o, Some(&p.index)).unwrap().1;
    assert!(
        md.starts_with("```lapsampler\ntarget_accept = ...\n```\n\nA setting of `adapt()`"),
        "{md}"
    );
}

#[test]
fn sampler_hover_ignores_keywords_blocks_and_numbers() {
    let p = project();
    for find in ["bloc", "block(a", "res", "0.", "impo"] {
        let (src, o) = in_sampler(find, 0);
        assert_eq!(
            hover(Language::Sampler, &src, o, Some(&p.index)),
            None,
            "{find}"
        );
    }
}

// ---- completion --------------------------------------------------------------

fn labels(items: &[Item]) -> Vec<&str> {
    items.iter().map(|i| i.label.as_str()).collect()
}

fn item<'a>(items: &'a [Item], label: &str) -> &'a Item {
    items
        .iter()
        .find(|i| i.label == label)
        .unwrap_or_else(|| panic!("no `{label}` in {:?}", labels(items)))
}

fn kernel_items_at(marked: &str) -> (String, Vec<Item>) {
    let (src, o) = at(marked);
    let items = completion(Language::Kernel, &src, o, None);
    (src, items)
}

#[test]
fn kernel_completion_in_a_step_offers_every_built_in() {
    let (src, items) =
        kernel_items_at("kernel k {\n  step {\n    vector[dim] z = std_n|\n  }\n}\n");
    let names: Vec<&str> = laplace_sampler_syntax::editor::builtins()
        .iter()
        .map(|b| b.name)
        .collect();
    assert_eq!(labels(&items), names);
    let rng = item(&items, "std_normal_rng");
    assert_eq!(&src[rng.replace.clone()], "std_n");
    assert_eq!(rng.insert, "std_normal_rng($0)");
    assert_eq!(rng.kind, ItemKind::Function);
    assert_eq!(
        rng.detail.as_deref(),
        Some("real std_normal_rng()  |  vector[n] std_normal_rng(int n)")
    );
    assert_eq!(item(&items, "q").insert, "q");
    assert_eq!(item(&items, "q").kind, ItemKind::Value);
    assert_eq!(item(&items, "propose").kind, ItemKind::Keyword);
}

#[test]
fn kernel_completion_in_tuning_offers_only_what_a_default_may_use() {
    let (_, items) = kernel_items_at("kernel k {\n  tuning {\n    real eps = |\n  }\n}\n");
    let offered = labels(&items);
    for allowed in ["dim", "rep_vector", "exp", "sqrt", "normal_lpdf"] {
        assert!(offered.contains(&allowed), "{allowed}: {offered:?}");
    }
    for refused in [
        "q",
        "lp",
        "grad",
        "metric",
        "log_density",
        "std_normal_rng",
        "propose",
    ] {
        assert!(!offered.contains(&refused), "{refused}: {offered:?}");
    }
}

#[test]
fn kernel_completion_offers_nothing_outside_blocks_in_comments_or_after_a_dot() {
    for marked in [
        "kernel |",
        "kernel k {\n  |",
        "kernel k {\n  step {\n    // q|\n  }\n}\n",
        "kernel k {\n  step {\n    /* q| */\n  }\n}\n",
        "kernel k {\n  step {\n    real x = t.|\n  }\n}\n",
    ] {
        assert_eq!(
            labels(&kernel_items_at(marked).1),
            Vec::<&str>::new(),
            "{marked}"
        );
    }
}

/// A sampler whose one kernel line is `line` (with the cursor `|`).
fn sampler_items_at(p: &TestProject, line: &str) -> (String, Vec<Item>) {
    let (src, o) = at(&format!(
        "sampler s {{\n  library {{ import langevin }}\n  kernel {{\n    {line}\n  }}\n}}\n"
    ));
    let items = completion(Language::Sampler, &src, o, Some(&p.index));
    (src, items)
}

const KERNELS: [&str; 12] = [
    "nuts",
    "rwm",
    "mala",
    "hmc",
    "langevin::barker",
    "langevin::broken",
    "langevin::mala",
    "./kernels/mine.lapkernel",
    "seq",
    "mix",
    "repeat",
    "adapt",
];

#[test]
fn sampler_completion_after_the_colon_offers_kernels_and_combinators() {
    let p = project();
    let (_, items) = sampler_items_at(&p, "rest : |");
    assert_eq!(labels(&items), KERNELS);
    assert_eq!(item(&items, "nuts").insert, "nuts($0)");
    assert_eq!(item(&items, "langevin::mala").insert, "langevin::mala");
    assert_eq!(
        item(&items, "langevin::mala").detail.as_deref(),
        Some(
            "kernel package `langevin` 0.1.0: Metropolis-adjusted Langevin algorithm with a \
             diagonal metric"
        )
    );
    assert_eq!(
        item(&items, "langevin::broken").detail.as_deref(),
        Some("kernel package `langevin` 0.1.0: does not check")
    );
    assert_eq!(item(&items, "seq").insert, "seq($0)");
    let sorted: Vec<&str> = {
        let mut v: Vec<&Item> = items.iter().collect();
        v.sort_by(|a, b| a.sort.cmp(&b.sort));
        v.iter().map(|i| i.label.as_str()).collect()
    };
    assert_eq!(&sorted[..4], ["hmc", "mala", "nuts", "rwm"]);
    assert_eq!(&sorted[8..], ["adapt", "mix", "repeat", "seq"]);
}

#[test]
fn sampler_completion_replaces_the_word_typed_so_far() {
    let p = project();
    for (line, typed) in [
        ("rest : langevin::m|", "langevin::m"),
        ("rest : langevin:|", "langevin:"),
        ("rest : ./ker|", "./ker"),
        ("rest : seq(rw|", "rw"),
    ] {
        let (src, items) = sampler_items_at(&p, line);
        assert_eq!(labels(&items), KERNELS, "{line}");
        assert_eq!(&src[items[0].replace.clone()], typed, "{line}");
    }
}

#[test]
fn sampler_completion_inside_combinators() {
    let p = project();
    let offered = |line: &str| {
        sampler_items_at(&p, line)
            .1
            .iter()
            .map(|i| i.label.clone())
            .collect::<Vec<_>>()
    };
    for line in [
        "rest : seq(rwm(), |",
        "rest : mix(1 : |",
        "rest : mix(1 : rwm(), 2 : |",
        "rest : repeat(3, |",
        "rest : adapt(|",
    ] {
        assert_eq!(offered(line), KERNELS, "{line}");
    }
    for line in ["rest : mix(|", "rest : repeat(|", "rest : mix(1 : rwm(), |"] {
        assert_eq!(offered(line), Vec::<String>::new(), "{line}");
    }
    assert_eq!(offered("rest : adapt(mala(), |"), ["target_accept"]);
    assert_eq!(
        offered("rest : adapt(mala(), target_accept = 0.6, |"),
        Vec::<String>::new()
    );
    assert_eq!(
        offered("rest : adapt(mala(), target_accept = |"),
        Vec::<String>::new()
    );
}

#[test]
fn sampler_completion_offers_a_kernels_settings_with_their_defaults() {
    let p = project();
    let (_, items) = sampler_items_at(&p, "rest : langevin::mala(|");
    assert_eq!(labels(&items), ["eps", "m"]);
    assert_eq!(item(&items, "eps").insert, "eps = ${1:1}");
    assert_eq!(
        item(&items, "eps").detail.as_deref(),
        Some("real<lower=0> eps = 1")
    );
    assert_eq!(item(&items, "m").insert, "m = ${1:rep_vector(1, dim)}");
    assert_eq!(item(&items, "eps").kind, ItemKind::Setting);

    let (_, items) = sampler_items_at(&p, "rest : ./kernels/mine.lapkernel(eps = 0.1, |");
    assert_eq!(labels(&items), ["m"]);
    let (_, items) = sampler_items_at(&p, "rest : hmc(steps = 3, |");
    assert_eq!(labels(&items), ["step_size", "jitter"]);
    assert_eq!(item(&items, "jitter").insert, "jitter = ${1:true}");
    let (_, items) = sampler_items_at(&p, "rest : mala(st|");
    assert_eq!(labels(&items), ["step_size"]);
    let (_, items) = sampler_items_at(&p, "rest : mala(step_size = |");
    assert_eq!(labels(&items), Vec::<&str>::new());
}

#[test]
fn sampler_completion_at_the_start_of_a_line_offers_block_and_rest() {
    let p = project();
    let (src, items) = sampler_items_at(&p, "bl|");
    assert_eq!(labels(&items), ["block", "rest"]);
    assert_eq!(&src[items[0].replace.clone()], "bl");
    assert_eq!(items[0].insert, "block(${1:param}) : $0");
    assert_eq!(items[1].insert, "rest : $0");
    // Parameter names need the model (Q8).
    for line in ["block(|", "block(a, |", "block(a) as |"] {
        assert_eq!(
            labels(&sampler_items_at(&p, line).1),
            Vec::<&str>::new(),
            "{line}"
        );
    }
}

#[test]
fn sampler_completion_offers_nothing_outside_kernel_lines() {
    let p = project();
    for marked in [
        "sampler s {\n  warmup { iterations = | }\n}\n",
        "sampler s {\n  library { import | }\n}\n",
        "sampler s {\n  |\n}\n",
        "sampler s {\n  kernel {\n    rest : nuts() // |\n  }\n}\n",
    ] {
        let (src, o) = at(marked);
        let items = completion(Language::Sampler, &src, o, Some(&p.index));
        assert_eq!(labels(&items), Vec::<&str>::new(), "{marked}");
    }
}

// ---- go-to-definition --------------------------------------------------------

#[test]
fn definition_of_package_and_local_kernels_is_their_file_at_the_kernel_name() {
    let p = project();
    let root = p.dir.parent().unwrap();
    for (find, file) in [
        ("langevin::ma", root.join("langevin/kernels/mala.lapkernel")),
        ("./kernels/mi", p.dir.join("kernels/mine.lapkernel")),
    ] {
        let (src, o) = in_sampler(find, 0);
        let Some(Definition::File(f, range)) = definition(&src, o, &p.index) else {
            panic!("{find}");
        };
        assert_eq!(f, file, "{find}");
        let text = std::fs::read_to_string(&f).unwrap();
        assert_eq!(&text[range.clone()], "mala", "{find}");
        assert!(text[..range.start].ends_with("\nkernel "), "{find}");
    }
}

#[test]
fn definition_of_a_built_in_is_its_implementation() {
    let p = project();
    for (find, b) in [("rw", BuiltIn::Rwm), ("nu", BuiltIn::Nuts)] {
        let (src, o) = in_sampler(find, 0);
        assert_eq!(definition(&src, o, &p.index), Some(Definition::BuiltIn(b)));
    }
}

#[test]
fn definition_of_a_path_outside_the_project_index() {
    let p = project();
    let outside = p.dir.parent().unwrap().join("elsewhere.lapkernel");
    std::fs::copy(p.dir.join("kernels/mine.lapkernel"), &outside).unwrap();
    let (src, o) = at("sampler s {\n  kernel {\n    rest : ../elsewhere.lap|kernel\n  }\n}\n");
    let Some(Definition::File(f, _)) = definition(&src, o, &p.index) else {
        panic!();
    };
    assert_eq!(f, p.dir.join("../elsewhere.lapkernel"));
}

#[test]
fn no_definition_for_combinators_settings_keywords_missing_files_or_comments() {
    let p = project();
    for find in ["ada", "se", "mala(ep", "target_acc", "bloc", "res"] {
        let (src, o) = in_sampler(find, 0);
        assert_eq!(definition(&src, o, &p.index), None, "{find}");
    }
    let (src, o) = at("sampler s {\n  kernel {\n    rest : ./nope.lap|kernel // ./kernels/mine.lapkernel\n  }\n}\n");
    assert_eq!(definition(&src, o, &p.index), None);
    let (src, o) =
        at("sampler s {\n  kernel {\n    rest : nuts() // ./kernels/mine.lap|kernel\n  }\n}\n");
    assert_eq!(definition(&src, o, &p.index), None);
}

#[test]
fn the_kernel_name_is_found_past_comments() {
    let src = "// kernel fake {\n/* kernel other { */\nkernel  real_one {\n}\n";
    assert_eq!(&src[kernel_name_range(src)], "real_one");
    assert_eq!(kernel_name_range("no kernel here?"), 10..14);
    assert_eq!(kernel_name_range(""), 0..0);
}

#[test]
fn a_built_ins_source_is_written_once_read_only() {
    let tmp = tempfile::tempdir().unwrap();
    let file = builtin_source_file(tmp.path(), BuiltIn::Mala).unwrap();
    assert!(file.ends_with("mala.rs"), "{}", file.display());
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        BuiltIn::Mala.source().1
    );
    assert!(std::fs::metadata(&file).unwrap().permissions().readonly());
    assert_eq!(
        builtin_source_file(tmp.path(), BuiltIn::Mala).unwrap(),
        file
    );
    // A damaged copy is rewritten.
    let mut perms = std::fs::metadata(&file).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&file, perms).unwrap();
    std::fs::write(&file, "edited").unwrap();
    builtin_source_file(tmp.path(), BuiltIn::Mala).unwrap();
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        BuiltIn::Mala.source().1
    );
}

// ---- signature help ----------------------------------------------------------

/// (label, each parameter's text, the active parameter's text).
fn signature_at(p: &TestProject, line: &str) -> Option<(String, Vec<String>, Option<String>)> {
    let (src, o) = at(&format!(
        "sampler s {{\n  library {{ import langevin }}\n  kernel {{\n    {line}\n  }}\n}}\n"
    ));
    let s = signature_help(&src, o, &p.index)?;
    let params: Vec<String> = s
        .parameters
        .iter()
        .map(|(r, _)| s.label[r.clone()].to_string())
        .collect();
    let active = s.active.map(|i| params[i].clone());
    Some((s.label, params, active))
}

#[test]
fn signature_help_for_combinators() {
    let p = project();
    let sig = |line: &str| signature_at(&p, line).unwrap();
    assert_eq!(
        sig("rest : seq(|"),
        (
            "seq(k1, k2, ...)".into(),
            vec!["k1".into(), "k2".into()],
            Some("k1".into())
        )
    );
    assert_eq!(sig("rest : seq(rwm(), mala(), |").2.as_deref(), Some("k2"));
    let (label, params, active) = sig("rest : adapt(mala(), |");
    assert_eq!(label, "adapt(k, target_accept = ...)");
    assert_eq!(params, ["k", "target_accept = ..."]);
    assert_eq!(active.as_deref(), Some("target_accept = ..."));
    assert_eq!(sig("rest : repeat(|").2.as_deref(), Some("n"));
    assert_eq!(sig("rest : repeat(3, |").2.as_deref(), Some("k"));
    let (_, params, _) = sig("rest : mix(|");
    assert_eq!(params, ["w1 : k1", "w2 : k2"]);
    let (_, params, _) = sig("rest : on(|");
    assert_eq!(params, ["block(p1, p2, ...)", "k"]);
}

#[test]
fn signature_help_for_kernel_calls_lists_settings_with_defaults() {
    let p = project();
    let (label, params, active) = signature_at(&p, "rest : langevin::mala(|").unwrap();
    assert_eq!(label, "langevin::mala(eps = 1, m = rep_vector(1, dim))");
    assert_eq!(params, ["eps = 1", "m = rep_vector(1, dim)"]);
    assert_eq!(active.as_deref(), Some("eps = 1"));
    let (label, _, active) = signature_at(&p, "rest : hmc(steps = 3, |").unwrap();
    assert_eq!(label, "hmc(step_size = 1, steps = 10, jitter = true)");
    assert_eq!(active.as_deref(), Some("step_size = 1"));
    for (line, want) in [
        ("rest : hmc(jit|", "jitter = true"),
        ("rest : hmc(steps = 3, jitter = fa|", "jitter = true"),
        ("rest : hmc(steps = |", "steps = 10"),
        (
            "rest : ./kernels/mine.lapkernel(m|",
            "m = rep_vector(1, dim)",
        ),
    ] {
        assert_eq!(
            signature_at(&p, line).unwrap().2.as_deref(),
            Some(want),
            "{line}"
        );
    }
    let (src, o) = at(
        "sampler s {\n  library { import langevin }\n  kernel {\n    rest : langevin::mala(|\n  }\n}\n",
    );
    let s = signature_help(&src, o, &p.index).unwrap();
    assert_eq!(
        s.documentation.as_deref(),
        Some("Metropolis-adjusted Langevin algorithm with a diagonal metric")
    );
    assert_eq!(
        s.parameters[0].1.as_deref(),
        Some("`@adapt(step_size, target_accept = 0.574)` Step size (adapted toward 0.574 acceptance)")
    );
}

#[test]
fn no_signature_help_outside_calls_on_blocks_or_broken_kernels() {
    let p = project();
    for line in [
        "rest : |",
        "block(|",
        "rest : seq(rwm(), mala())|",
        "rest : langevin::broken(|",
        "rest : nuts() // seq(|",
    ] {
        assert_eq!(signature_at(&p, line), None, "{line}");
    }
}
