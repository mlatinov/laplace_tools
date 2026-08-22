//! Stan's built-in function names, bundled as a static data file rather than
//! derived by parsing Stan's math library (see data/stan-builtins.json).
//! Used to classify a bare `func(` call site as "builtin" rather than
//! "user-defined" or "unresolved".

use std::collections::HashSet;

use once_cell::sync::Lazy;
use serde::Deserialize;

const BUILTINS_JSON: &str = include_str!("../data/stan-builtins.json");

#[derive(Deserialize)]
struct BuiltinsFile {
    functions: Vec<String>,
}

static STAN_BUILTINS: Lazy<HashSet<String>> = Lazy::new(|| {
    let parsed: BuiltinsFile =
        serde_json::from_str(BUILTINS_JSON).expect("bundled stan-builtins.json must be valid JSON");
    parsed.functions.into_iter().collect()
});

pub fn is_builtin(name: &str) -> bool {
    STAN_BUILTINS.contains(name)
}
