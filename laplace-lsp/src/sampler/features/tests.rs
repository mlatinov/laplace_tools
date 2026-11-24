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
