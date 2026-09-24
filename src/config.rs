//! `computed.toml` and the `use` loader. A repository names recipes, each a
//! loader and its attributes, and a region writes `use recipe=NAME` for the
//! opener the recipe stands for. The expansion is an opener like any other:
//! its paths resolve against the template's region root, its canonical form
//! goes into the input sum, and an exec recipe needs trust as exec does.
//!
//! ```toml
//! [recipe.adrs]
//! loader = "exec"
//! cmd = "../scripts/adr-index.sh"
//! inputs = "docs/adr/*.md"
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::marker::{self, File, Opener, Segment};

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
    for dir in start.ancestors() {
        let candidate = dir.join(FILE_NAME);
        if candidate.is_file() {
            return Ok(candidate);
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
            if key != "recipe" {
                return Err(at(format!(
                    "unknown key {key:?}; recipes go under [recipe.NAME]"
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

    /// The opener a `use` region stands for, marked as expanded from it.
    pub fn expand(&self, opener: &Opener, line: usize) -> Result<Opener, String> {
        let name = opener.attr("recipe").unwrap_or_default();
        let Some(recipe) = self.recipes.get(name) else {
            return Err(format!(
                "recipe={name}: {} has no [recipe.{name}]",
                self.path.display()
            ));
        };
        let region: Vec<(&str, &str)> = opener.common_attrs().collect();
        marker::opener(line, &recipe.expand(&region))
            .map(|o| o.expanded_from(opener))
            .map_err(|e| format!("[recipe.{name}]: {}", e.message))
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

/// What expanding a file's `use` regions came to.
#[derive(Debug, Default)]
pub struct Expansion {
    /// The canonical `computed.toml` read, when there was one to read.
    pub read: Option<PathBuf>,
    /// Per region line, why its recipe could not be expanded.
    pub errors: BTreeMap<usize, String>,
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
            Ok(c) => c.expand(&region.opener, region.line),
            Err(e) => Err(e.clone()),
        };
        match expanded {
            Ok(opener) => region.opener = opener,
            Err(e) => {
                expansion.errors.insert(region.line, e);
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

    #[test]
    fn a_recipe_expands_to_its_loader_attributes_by_key_then_common_ones() {
        let c = config(
            "[recipe.adrs]\nloader = \"exec\"\ninputs = \"docs/adr/*.md\"\ncmd = \"../scripts/adr index.sh\"\ntimeout = 5\nlang = \"md\"\nas = \"fence\"\n",
        )
        .unwrap();
        let o = c
            .expand(&use_opener("use recipe=adrs lang=text name=d"), 1)
            .unwrap();
        assert_eq!(
            o.canonical(),
            "<!-- computed exec cmd=\"../scripts/adr index.sh\" inputs=docs/adr/*.md timeout=5 as=fence lang=text name=d -->"
        );
        assert_eq!(o.sink, marker::Sink::Fence);
        assert_eq!(o.lang, "text");
        assert_eq!(o.name.as_deref(), Some("d"));
        assert_eq!(
            marker::rendered_opener(&o),
            "<!-- computed use recipe=adrs lang=text name=d | do not edit; run computed -->"
        );
    }

    #[test]
    fn the_target_loaders_default_sink_applies() {
        let c = config("[recipe.t]\nloader = \"tree\"\ndepth = 2\nall = true\n").unwrap();
        let o = c.expand(&use_opener("use recipe=t"), 1).unwrap();
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
        let e = c.expand(&use_opener("use recipe=x"), 1).unwrap_err();
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
}
