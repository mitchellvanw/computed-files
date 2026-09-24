//! `computed.toml` and the `use` loader. A repository names recipes, each a
//! loader and its attributes, and a region writes `use recipe=NAME` for the
//! opener the recipe stands for. The expansion is an opener like any other:
//! its paths resolve against the template's region root, its canonical form
//! goes into the input sum, and an exec recipe needs trust as exec does.
//! The file may also hold a `[discover]` table: the code files, by
//! extension or name, that discovery reads beside Markdown ([`Discover`]).
//!
//! ```toml
//! [recipe.adrs]
//! loader = "exec"
//! cmd = "../scripts/adr-index.sh"
//! inputs = "docs/adr/*.md"
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::marker::{self, File, Opener, Segment, Syntax};

pub const FILE_NAME: &str = "computed.toml";

/// The common attributes a recipe may set, the table sink's `delim=` and
/// `from=` among them; a region's own override them. `name=` is per file,
/// so only a region gives it.
const RECIPE_COMMON: &[&str] = &["as", "lang", "on-stale", "max-lines", "delim", "from"];

/// A parsed, validated `computed.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub path: PathBuf,
    recipes: BTreeMap<String, Recipe>,
}

/// One `[recipe.NAME]` table.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Recipe {
    loader: String,
    /// The loader's own attributes and flags, sorted by key.
    own: Vec<Entry>,
    /// The common attributes the recipe sets, sorted by key.
    common: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Entry {
    Flag(String),
    Attr(String, String),
}

/// The `computed.toml` a template under `region_root` reads: the nearest
/// one walking up from the region root, the repository root the last
/// directory looked in. Outside a repository only the region root is.
pub fn find(region_root: &Path, repo_root: Option<&Path>) -> Result<PathBuf, String> {
    let start = region_root
        .canonicalize()
        .map_err(|e| format!("region root: {e}"))?;
    let bound = repo_root.unwrap_or(&start);
    for dir in start.ancestors() {
        let candidate = dir.join(FILE_NAME);
        if candidate.is_file() {
            return inside(&candidate, bound);
        }
        if repo_root.is_none_or(|r| r == dir) {
            break;
        }
    }
    Err(match repo_root {
        Some(r) => format!(
            "no {FILE_NAME} in {} or a directory above it up to {}",
            start.display(),
            r.display()
        ),
        None => format!("no {FILE_NAME} in {}", start.display()),
    })
}

/// `path`, a `computed.toml`, when it lies inside `bound` (canonical). A
/// symlink out of the repository is refused unread, so no error message
/// can show a line of a file outside it.
fn inside(path: &Path, bound: &Path) -> Result<PathBuf, String> {
    let target = path
        .canonicalize()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if !target.starts_with(bound) {
        return Err(format!("{} escapes {}", path.display(), bound.display()));
    }
    Ok(path.to_path_buf())
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Config::parse(path, &text)
    }

    /// Parses and validates every recipe: one that could not be expanded
    /// makes the whole file an error, as a malformed template is one.
    pub fn parse(path: &Path, text: &str) -> Result<Config, String> {
        let at = |m: String| format!("{}: {m}", path.display());
        let table: toml::Table = text.parse().map_err(|e| at(format!("{e}")))?;
        let mut recipes = BTreeMap::new();
        for (key, value) in table {
            if key == "discover" {
                Discover::parse(value).map_err(&at)?;
                continue;
            }
            if key != "recipe" {
                return Err(at(format!(
                    "unknown key {key:?}; recipes go under [recipe.NAME], and discovery under [discover]"
                )));
            }
            let toml::Value::Table(named) = value else {
                return Err(at("recipe is not a table of [recipe.NAME] tables".into()));
            };
            for (name, value) in named {
                let recipe = Recipe::parse(&name, value).map_err(&at)?;
                marker::opener(0, &recipe.expand(&[]))
                    .map_err(|e| at(format!("[recipe.{name}]: {}", e.message)))?;
                recipes.insert(name, recipe);
            }
        }
        Ok(Config {
            path: path.to_path_buf(),
            recipes,
        })
    }

    /// The opener a `use` region in a file of `syntax`, inside a line when
    /// `inline`, stands for, marked as expanded from it.
    pub fn expand(
        &self,
        opener: &Opener,
        line: usize,
        syntax: Syntax,
        inline: bool,
    ) -> Result<Opener, String> {
        let name = opener.attr("recipe").unwrap_or_default();
        let Some(recipe) = self.recipes.get(name) else {
            return Err(format!(
                "recipe={name}: {} has no [recipe.{name}]",
                self.path.display()
            ));
        };
        let region: Vec<(&str, &str)> = opener.common_attrs().collect();
        marker::opener(line, &recipe.expand(&region))
            .map_err(|e| e.message)
            .and_then(|o| o.placed(syntax, inline))
            .map(|o| o.expanded_from(opener))
            .map_err(|m| format!("[recipe.{name}]: {m}"))
    }
}

impl Recipe {
    fn parse(name: &str, value: toml::Value) -> Result<Recipe, String> {
        let toml::Value::Table(table) = value else {
            return Err(format!("recipe.{name} is not a table"));
        };
        let mut loader = None;
        let mut own = Vec::new();
        let mut common = Vec::new();
        for (key, value) in table {
            let value = match value {
                toml::Value::String(s) => s,
                toml::Value::Integer(n) => n.to_string(),
                toml::Value::Boolean(true) if key != "loader" && !marker::is_common(&key) => {
                    own.push(Entry::Flag(key));
                    continue;
                }
                toml::Value::Boolean(false) => {
                    return Err(format!(
                        "[recipe.{name}] {key} = false: set a flag with true, or leave it out"
                    ));
                }
                _ => {
                    return Err(format!(
                        "[recipe.{name}] {key}: expected a string, a whole number or true"
                    ));
                }
            };
            match key.as_str() {
                "loader" if value == "use" => {
                    return Err(format!(
                        "[recipe.{name}] loader=use: a recipe cannot use another recipe"
                    ));
                }
                "loader" => loader = Some(value),
                "name" => {
                    return Err(format!(
                        "[recipe.{name}] name=: a region names itself; a recipe cannot"
                    ));
                }
                k if RECIPE_COMMON.contains(&k) => common.push((key, value)),
                _ => own.push(Entry::Attr(key, value)),
            }
        }
        let loader = loader.ok_or_else(|| format!("[recipe.{name}] needs loader="))?;
        own.sort_by(|a, b| a.key().cmp(b.key()));
        common.sort();
        Ok(Recipe {
            loader,
            own,
            common,
        })
    }

    /// The expanded opener's content: the loader, its own attributes and
    /// flags by key, the recipe's common attributes the region does not
    /// give, then the region's common attributes as written.
    fn expand(&self, region: &[(&str, &str)]) -> String {
        let mut out = self.loader.clone();
        for entry in &self.own {
            out.push(' ');
            match entry {
                Entry::Flag(f) => out.push_str(f),
                Entry::Attr(k, v) => out.push_str(&format!("{k}={}", marker::quote(v))),
            }
        }
        let recipe = self
            .common
            .iter()
            .filter(|(k, _)| !region.iter().any(|(r, _)| r == k))
            .map(|(k, v)| (k.as_str(), v.as_str()));
        for (k, v) in recipe.chain(region.iter().copied()) {
            out.push_str(&format!(" {k}={}", marker::quote(v)));
        }
        out
    }
}

impl Entry {
    fn key(&self) -> &str {
        match self {
            Entry::Flag(k) | Entry::Attr(k, _) => k,
        }
    }
}

/// `[discover]`: the files beyond Markdown that discovery reads, by
/// extension or by name. Each must have a comment syntax.
///
/// ```toml
/// [discover]
/// extensions = ["rs", "html"]
/// names = ["Makefile"]
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Discover {
    /// Lowercase, without the dot.
    extensions: Vec<String>,
    names: Vec<String>,
}

impl Discover {
    fn parse(value: toml::Value) -> Result<Discover, String> {
        let toml::Value::Table(table) = value else {
            return Err("discover is not a table; write [discover]".into());
        };
        let mut discover = Discover::default();
        for (key, value) in table {
            let list = |value: toml::Value| -> Result<Vec<String>, String> {
                let toml::Value::Array(items) = value else {
                    return Err(format!("[discover] {key}: expected a list of strings"));
                };
                items
                    .into_iter()
                    .map(|v| match v {
                        toml::Value::String(s) => Ok(s),
                        _ => Err(format!("[discover] {key}: expected a list of strings")),
                    })
                    .collect()
            };
            match key.as_str() {
                "extensions" => {
                    for ext in list(value)? {
                        let ext = ext.trim_start_matches('.').to_ascii_lowercase();
                        if Syntax::of_extension(&ext).is_none() {
                            return Err(format!(
                                "[discover] extensions: .{ext} has no comment syntax the tool knows; it knows {}",
                                Syntax::supported()
                            ));
                        }
                        discover.extensions.push(ext);
                    }
                }
                "names" => {
                    for name in list(value)? {
                        if Syntax::of_name(&name).is_none() {
                            return Err(format!(
                                "[discover] names: {name} has no comment syntax the tool knows; it knows {}",
                                Syntax::supported()
                            ));
                        }
                        discover.names.push(name);
                    }
                }
                _ => {
                    return Err(format!(
                        "[discover] {key}: unknown key; it takes extensions and names"
                    ));
                }
            }
        }
        Ok(discover)
    }

    /// Whether discovery reads the walked file at `path`: Markdown always,
    /// anything else when its extension or name is listed.
    pub fn selects(&self, path: &Path) -> bool {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            return false;
        };
        let ext = path.extension().and_then(|e| e.to_str());
        ext.is_some_and(|e| e == "md" || e == "markdown")
            || ext.is_some_and(|e| self.extensions.contains(&e.to_ascii_lowercase()))
            || self.names.iter().any(|n| n == name)
    }
}

/// The `[discover]` table of the `computed.toml` in `dir`, empty when there
/// is none. Only that table is read: a recipe's error belongs to the
/// regions that use it, not to discovery.
pub fn discovery(dir: &Path) -> Result<Discover, String> {
    let path = dir.join(FILE_NAME);
    if !path.exists() {
        return Ok(Discover::default());
    }
    let bound = dir
        .canonicalize()
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    inside(&path, &bound)?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Discover::default()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let at = |m: String| format!("{}: {m}", path.display());
    let mut table: toml::Table = text.parse().map_err(|e| at(format!("{e}")))?;
    match table.remove("discover") {
        Some(value) => Discover::parse(value).map_err(at),
        None => Ok(Discover::default()),
    }
}

/// What expanding a file's `use` regions came to.
#[derive(Debug, Clone, Default)]
pub struct Expansion {
    /// The canonical `computed.toml` read, when there was one to read.
    pub read: Option<PathBuf>,
    /// Per region place (`Region::at`), why its recipe could not be expanded.
    pub errors: BTreeMap<(usize, Option<usize>), String>,
}

/// Replaces the opener of every `use` region of `file` with the opener its
/// recipe stands for, found from `region_root`. The file is read only when a
/// region uses a recipe. A region whose recipe does not expand keeps its
/// `use` opener and its error.
pub fn expand(file: &mut File, region_root: &Path, repo_root: Option<&Path>) -> Expansion {
    let mut expansion = Expansion::default();
    let mut config: Option<Result<Config, String>> = None;
    for segment in &mut file.segments {
        let Segment::Region(region) = segment else {
            continue;
        };
        if region.opener.loader != "use" {
            continue;
        }
        let config = config.get_or_insert_with(|| {
            let path = find(region_root, repo_root)?;
            expansion.read = path.canonicalize().ok();
            Config::load(&path)
        });
        let expanded = match config {
            Ok(c) => c.expand(
                &region.opener,
                region.line,
                region.syntax,
                region.column.is_some(),
            ),
            Err(e) => Err(e.clone()),
        };
        match expanded {
            Ok(opener) => region.opener = opener,
            Err(e) => {
                expansion.errors.insert(region.at(), e);
            }
        }
    }
    expansion
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(text: &str) -> Result<Config, String> {
        Config::parse(Path::new("computed.toml"), text)
    }

    fn use_opener(content: &str) -> Opener {
        marker::opener(1, content).unwrap()
    }

    impl Config {
        fn expand_md(&self, opener: &Opener, line: usize) -> Result<Opener, String> {
            self.expand(opener, line, Syntax::Markdown, false)
        }
    }

    #[test]
    fn a_recipe_expands_to_its_loader_attributes_by_key_then_common_ones() {
        let c = config(
            "[recipe.adrs]\nloader = \"exec\"\ninputs = \"docs/adr/*.md\"\ncmd = \"../scripts/adr index.sh\"\ntimeout = 5\nlang = \"md\"\nas = \"fence\"\n",
        )
        .unwrap();
        let o = c
            .expand_md(&use_opener("use recipe=adrs lang=text name=d"), 1)
            .unwrap();
        assert_eq!(
            o.canonical(),
            "<!-- computed exec cmd=\"../scripts/adr index.sh\" inputs=docs/adr/*.md timeout=5 as=fence lang=text name=d -->"
        );
        assert_eq!(o.sink, marker::Sink::Fence);
        assert_eq!(o.lang, "text");
        assert_eq!(o.name.as_deref(), Some("d"));
        assert_eq!(
            marker::rendered_opener(&o, marker::Comment::HTML),
            "<!-- computed use recipe=adrs lang=text name=d | do not edit; run computed -->"
        );
    }

    #[test]
    fn the_target_loaders_default_sink_applies() {
        let c = config("[recipe.t]\nloader = \"tree\"\ndepth = 2\nall = true\n").unwrap();
        let o = c.expand_md(&use_opener("use recipe=t"), 1).unwrap();
        assert_eq!(o.canonical(), "<!-- computed tree all depth=2 -->");
        assert_eq!(o.sink, marker::Sink::Fence);
    }

    #[test]
    fn every_recipe_is_validated_when_the_file_is_read() {
        for (text, needle) in [
            (
                "[recipe.a]\nloader = \"exec\"\ncmd = \"x\"\n",
                "inputs= or the volatile flag",
            ),
            ("[recipe.a]\nloader = \"csv\"\n", "unknown loader"),
            (
                "[recipe.a]\nloader = \"tree\"\nvolatile = true\n",
                "unknown flag",
            ),
            ("[recipe.a]\nloader = \"tree\"\nall = false\n", "true"),
            (
                "[recipe.a]\nloader = \"tree\"\nmax-lines = 0\n",
                "at least 1",
            ),
            (
                "[recipe.a]\nloader = \"tree\"\ndepth = 1.5\n",
                "expected a string",
            ),
            ("recipe = 1\n", "table"),
            ("[recipe]\na = 1\n", "recipe.a is not a table"),
        ] {
            let e = config(text).unwrap_err();
            assert!(e.contains(needle), "{text}: {e}");
            assert!(e.starts_with("computed.toml: "), "{e}");
        }
    }

    #[test]
    fn a_missing_recipe_names_the_file_it_looked_in() {
        let c = config("").unwrap();
        let e = c.expand_md(&use_opener("use recipe=x"), 1).unwrap_err();
        assert_eq!(e, "recipe=x: computed.toml has no [recipe.x]");
    }

    #[test]
    fn find_walks_up_to_the_repository_root_and_no_further() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let repo = root.join("repo");
        std::fs::create_dir_all(repo.join("a/b")).unwrap();
        std::fs::write(root.join(FILE_NAME), "").unwrap();
        assert!(find(&repo.join("a/b"), Some(&repo)).is_err());
        std::fs::write(repo.join(FILE_NAME), "").unwrap();
        assert_eq!(
            find(&repo.join("a/b"), Some(&repo)),
            Ok(repo.join(FILE_NAME))
        );
        std::fs::write(repo.join("a").join(FILE_NAME), "").unwrap();
        assert_eq!(
            find(&repo.join("a/b"), Some(&repo)),
            Ok(repo.join("a").join(FILE_NAME))
        );
        assert!(
            find(&repo.join("a/b"), None).is_err(),
            "outside a repository only the region root"
        );
        assert_eq!(
            find(&repo.join("a"), None),
            Ok(repo.join("a").join(FILE_NAME))
        );
    }

    #[test]
    fn a_computed_toml_that_links_out_of_the_repository_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let repo = root.join("repo");
        std::fs::create_dir_all(repo.join("a")).unwrap();
        std::fs::write(root.join("creds"), "ghp_SECRET\n").unwrap();
        std::os::unix::fs::symlink(root.join("creds"), repo.join(FILE_NAME)).unwrap();
        let e = find(&repo.join("a"), Some(&repo)).unwrap_err();
        assert!(e.contains("escapes"), "{e}");
        let e = discovery(&repo).unwrap_err();
        assert!(e.contains("escapes") && !e.contains("SECRET"), "{e}");
        // A link that stays inside is read as the file it names.
        std::fs::write(
            repo.join("real.toml"),
            "[discover]\nnames = [\"Makefile\"]\n",
        )
        .unwrap();
        std::fs::remove_file(repo.join(FILE_NAME)).unwrap();
        std::os::unix::fs::symlink(repo.join("real.toml"), repo.join(FILE_NAME)).unwrap();
        assert!(discovery(&repo).unwrap().selects(Path::new("Makefile")));
        assert_eq!(find(&repo.join("a"), Some(&repo)), Ok(repo.join(FILE_NAME)));
    }
}
