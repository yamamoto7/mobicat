//! Configuration files (docs/requirements.md §11).
//!
//! Two layers: the user's `~/.config/mobicat/config.toml` (`%APPDATA%\mobicat\config.toml`
//! on Windows), then `.mobicat.toml` found by walking up from the current
//! directory. Later layers win; command-line flags win over both.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use toml::{Table, Value};

pub const REPO_FILE: &str = ".mobicat.toml";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    pub app: Option<String>,
    pub android: Option<String>,
    pub ios: Option<String>,
    pub level: Option<String>,
    pub query: Option<String>,
    pub all: Option<bool>,
    pub wrap: Option<bool>,
    pub buffer: Option<usize>,
    /// In file order: (name, query, extends).
    pub profiles: Vec<(String, Option<String>, Option<String>)>,
    pub noise_tags: Vec<String>,
    pub noise_keep: Vec<String>,
    /// Files that were read, for messages.
    pub sources: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

pub fn user_path() -> Option<PathBuf> {
    if cfg!(windows) {
        return std::env::var_os("APPDATA")
            .map(|d| PathBuf::from(d).join("mobicat").join("config.toml"));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("mobicat").join("config.toml"))
}

/// Where mobicat keeps what it remembers between runs (query history, recent
/// apps): `~/.local/state/mobicat` (`%LOCALAPPDATA%\mobicat` on Windows).
pub fn state_dir() -> Option<PathBuf> {
    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("mobicat"));
    }
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("state"))
        })?;
    Some(base.join("mobicat"))
}

/// The nearest `.mobicat.toml` in `dir` or its ancestors.
pub fn find_repo_file(dir: &Path) -> Option<PathBuf> {
    dir.ancestors()
        .map(|d| d.join(REPO_FILE))
        .find(|p| p.is_file())
}

/// Load and merge the user and repository files.
pub fn load() -> Result<Config> {
    let mut cfg = Config::default();
    if let Some(p) = user_path().filter(|p| p.is_file()) {
        cfg.merge_file(&p)?;
    }
    if let Some(p) = std::env::current_dir()
        .ok()
        .and_then(|d| find_repo_file(&d))
    {
        cfg.merge_file(&p)?;
    }
    Ok(cfg)
}

impl Config {
    pub fn merge_file(&mut self, path: &Path) -> Result<()> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("could not read {}", path.display()))?;
        self.merge_str(&text, &path.display().to_string())
            .with_context(|| format!("in {}", path.display()))?;
        self.sources.push(path.to_owned());
        Ok(())
    }

    pub fn merge_str(&mut self, text: &str, name: &str) -> Result<()> {
        let table: Table = text
            .parse()
            .map_err(|e: toml::de::Error| anyhow::anyhow!("{}", e.message()))?;
        let mut warn = |key: &str| {
            self.warnings
                .push(format!("{name}: unknown setting `{key}` (ignored)"))
        };
        let mut out = Partial::default();
        for (k, v) in &table {
            match k.as_str() {
                "app" => out.app = Some(string(v, k)?),
                "apps" => {
                    for (k2, v2) in table_of(v, k)? {
                        match k2.as_str() {
                            "android" => out.android = Some(string(v2, "apps.android")?),
                            "ios" => out.ios = Some(string(v2, "apps.ios")?),
                            other => warn(&format!("apps.{other}")),
                        }
                    }
                }
                "defaults" => {
                    for (k2, v2) in table_of(v, k)? {
                        let key = format!("defaults.{k2}");
                        match k2.as_str() {
                            "level" => {
                                let l = string(v2, &key)?;
                                if mobicat_core::Level::parse(&l).is_none() {
                                    bail!(
                                        "`{key}` must be verbose, debug, info, warn, error or fatal, not `{l}`"
                                    );
                                }
                                out.level = Some(l);
                            }
                            "query" => out.query = Some(string(v2, &key)?),
                            "all" => out.all = Some(boolean(v2, &key)?),
                            "wrap" => out.wrap = Some(boolean(v2, &key)?),
                            "buffer" => {
                                let n =
                                    v2.as_integer().filter(|n| *n >= 1000).with_context(|| {
                                        format!(
                                            "`{key}` must be a number of entries, at least 1000"
                                        )
                                    })?;
                                out.buffer = Some(n as usize);
                            }
                            _ => warn(&key),
                        }
                    }
                }
                "profiles" => {
                    for (pname, pv) in table_of(v, k)? {
                        let mut query = None;
                        let mut extends = None;
                        for (k3, v3) in table_of(pv, &format!("profiles.{pname}"))? {
                            let key = format!("profiles.{pname}.{k3}");
                            match k3.as_str() {
                                "query" => query = Some(string(v3, &key)?),
                                "extends" => extends = Some(string(v3, &key)?),
                                _ => warn(&key),
                            }
                        }
                        out.profiles.push((pname.clone(), query, extends));
                    }
                }
                "noise" => {
                    for (k2, v2) in table_of(v, k)? {
                        if k2 != "android" {
                            warn(&format!("noise.{k2}"));
                            continue;
                        }
                        for (k3, v3) in table_of(v2, "noise.android")? {
                            let key = format!("noise.android.{k3}");
                            match k3.as_str() {
                                "tags" => out.noise_tags = strings(v3, &key)?,
                                "keep" => out.noise_keep = strings(v3, &key)?,
                                _ => warn(&key),
                            }
                        }
                    }
                }
                other => warn(other),
            }
        }
        self.apply(out);
        Ok(())
    }

    fn apply(&mut self, p: Partial) {
        macro_rules! over {
            ($($f:ident),*) => { $( if p.$f.is_some() { self.$f = p.$f; } )* };
        }
        over!(app, android, ios, level, query, all, wrap, buffer);
        for (name, q, ext) in p.profiles {
            match self.profiles.iter_mut().find(|(n, _, _)| *n == name) {
                Some(slot) => *slot = (name, q, ext),
                None => self.profiles.push((name, q, ext)),
            }
        }
        self.noise_tags.extend(p.noise_tags);
        self.noise_keep.extend(p.noise_keep);
    }

    /// A profile's query with its `extends` chain applied (parent AND child).
    pub fn profile_query(&self, name: &str) -> Result<String> {
        let mut seen: Vec<String> = Vec::new();
        let mut queries: Vec<String> = Vec::new();
        let mut cur = name.to_owned();
        loop {
            if seen.contains(&cur) {
                bail!(
                    "profile `{name}`: `extends` loops ({} → {cur})",
                    seen.join(" → ")
                );
            }
            let Some((_, q, ext)) = self.profiles.iter().find(|(p, _, _)| *p == cur) else {
                if let Some(last) = seen.last() {
                    bail!("profile `{last}` extends unknown profile `{cur}`");
                }
                let known: Vec<_> = self.profiles.iter().map(|(n, _, _)| n.as_str()).collect();
                let known = if known.is_empty() {
                    "none defined".to_owned()
                } else {
                    format!("defined: {}", known.join(", "))
                };
                bail!("no profile `{name}` ({known})");
            };
            seen.push(cur.clone());
            if let Some(q) = q {
                queries.push(q.clone());
            }
            match ext {
                Some(e) => cur = e.clone(),
                None => break,
            }
        }
        queries.reverse(); // parents first
        Ok(and_all(queries.iter().map(String::as_str)))
    }

    /// All profiles in file order with resolved queries.
    pub fn resolved_profiles(&self) -> Result<Vec<(String, String)>> {
        self.profiles
            .iter()
            .map(|(n, _, _)| Ok((n.clone(), self.profile_query(n)?)))
            .collect()
    }
}

/// AND queries together, parenthesizing so `|` inside one cannot leak.
pub fn and_all<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let parts: Vec<&str> = parts
        .into_iter()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    match parts.len() {
        0 => String::new(),
        1 => parts[0].to_owned(),
        _ => parts
            .iter()
            .map(|p| format!("({p})"))
            .collect::<Vec<_>>()
            .join(" "),
    }
}

#[derive(Default)]
struct Partial {
    app: Option<String>,
    android: Option<String>,
    ios: Option<String>,
    level: Option<String>,
    query: Option<String>,
    all: Option<bool>,
    wrap: Option<bool>,
    buffer: Option<usize>,
    profiles: Vec<(String, Option<String>, Option<String>)>,
    noise_tags: Vec<String>,
    noise_keep: Vec<String>,
}

fn string(v: &Value, key: &str) -> Result<String> {
    v.as_str()
        .map(str::to_owned)
        .with_context(|| format!("`{key}` must be a string"))
}

fn boolean(v: &Value, key: &str) -> Result<bool> {
    v.as_bool()
        .with_context(|| format!("`{key}` must be true or false"))
}

fn strings(v: &Value, key: &str) -> Result<Vec<String>> {
    v.as_array()
        .and_then(|a| {
            a.iter()
                .map(|x| x.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
        })
        .with_context(|| format!("`{key}` must be a list of strings"))
}

fn table_of<'a>(v: &'a Value, key: &str) -> Result<&'a Table> {
    v.as_table()
        .with_context(|| format!("`{key}` must be a table"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
app = "com.example.app"
[apps]
android = "com.example.app.dev"

[defaults]
level = "debug"
query = "-tag:Chatty"
wrap = true

[profiles.network]
query = "tag:OkHttp | category:network"

[profiles.network-errors]
extends = "network"
query = "level:error"

[noise.android]
tags = ["Chatty", "OkHttp*"]
keep = ["chromium"]
"#;

    fn parse(text: &str) -> Config {
        let mut c = Config::default();
        c.merge_str(text, "test").unwrap();
        c
    }

    #[test]
    fn reads_every_setting() {
        let c = parse(EXAMPLE);
        assert_eq!(c.app.as_deref(), Some("com.example.app"));
        assert_eq!(c.android.as_deref(), Some("com.example.app.dev"));
        assert_eq!(c.level.as_deref(), Some("debug"));
        assert_eq!(c.wrap, Some(true));
        assert_eq!(c.noise_tags, vec!["Chatty", "OkHttp*"]);
        assert!(c.warnings.is_empty(), "{:?}", c.warnings);
        let names: Vec<_> = c.profiles.iter().map(|(n, _, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec!["network", "network-errors"],
            "file order is kept for keys 1–9"
        );
    }

    #[test]
    fn extends_ands_parent_first_and_parenthesizes() {
        let c = parse(EXAMPLE);
        assert_eq!(
            c.profile_query("network").unwrap(),
            "tag:OkHttp | category:network"
        );
        let q = c.profile_query("network-errors").unwrap();
        assert_eq!(q, "(tag:OkHttp | category:network) (level:error)");
        mobicat_core::Query::parse(&q).expect("resolved query parses");
    }

    #[test]
    fn profile_errors() {
        let c = parse("[profiles.a]\nextends = \"b\"\n[profiles.b]\nextends = \"a\"\n");
        assert!(
            c.profile_query("a")
                .unwrap_err()
                .to_string()
                .contains("loops")
        );
        let c = parse("[profiles.a]\nextends = \"zzz\"\n");
        assert!(
            c.profile_query("a")
                .unwrap_err()
                .to_string()
                .contains("unknown profile `zzz`")
        );
        assert!(
            c.profile_query("nope")
                .unwrap_err()
                .to_string()
                .contains("defined: a")
        );
    }

    #[test]
    fn later_layers_win() {
        let mut c = parse(EXAMPLE);
        c.merge_str(
            "[defaults]\nlevel = \"warn\"\n[profiles.network]\nquery = \"tag:Retrofit\"\n",
            "repo",
        )
        .unwrap();
        assert_eq!(c.level.as_deref(), Some("warn"));
        assert_eq!(
            c.query.as_deref(),
            Some("-tag:Chatty"),
            "untouched keys stay"
        );
        assert_eq!(c.profile_query("network").unwrap(), "tag:Retrofit");
        assert_eq!(c.profiles.len(), 2);
    }

    #[test]
    fn unknown_keys_warn_and_bad_values_fail() {
        let c = parse("colour = \"red\"\n[defaults]\nlevels = \"x\"\n");
        assert_eq!(c.warnings.len(), 2);
        assert!(c.warnings[0].contains("`colour`"));
        let mut c = Config::default();
        let err = c
            .merge_str("[defaults]\nlevel = \"loud\"\n", "t")
            .unwrap_err();
        assert!(err.to_string().contains("must be verbose"));
        let err = c.merge_str("app = 3\n", "t").unwrap_err();
        assert!(err.to_string().contains("must be a string"));
        let err = c.merge_str("app = \n", "t").unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn finds_repo_file_upward() {
        let root = std::env::temp_dir().join(format!("mobicat-cfg-{}", std::process::id()));
        let deep = root.join("a/b");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(root.join(REPO_FILE), "app = \"x\"\n").unwrap();
        assert_eq!(find_repo_file(&deep), Some(root.join(REPO_FILE)));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn and_all_skips_empty() {
        assert_eq!(and_all(["", " a "]), "a");
        assert_eq!(and_all(["a | b", "c"]), "(a | b) (c)");
        assert_eq!(and_all([""]), "");
    }
}
