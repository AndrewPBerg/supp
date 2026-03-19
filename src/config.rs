use serde::Deserialize;
use std::path::PathBuf;

// ── Public config types (unchanged API) ─────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Config {
    pub global: GlobalConfig,
    pub diff: DiffConfig,
    pub pick: PickConfig,
    pub limits: LimitsConfig,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct GlobalConfig {
    pub no_copy: bool,
    pub no_color: bool,
    pub depth: usize,
    pub mode: String,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct DiffConfig {
    pub context_lines: u32,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct PickConfig {
    pub preview_lines: usize,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct LimitsConfig {
    pub max_untracked_file_size_mb: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            global: GlobalConfig::default(),
            diff: DiffConfig::default(),
            pick: PickConfig::default(),
            limits: LimitsConfig::default(),
        }
    }
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            no_copy: false,
            no_color: false,
            depth: 2,
            mode: "full".to_string(),
        }
    }
}

impl Default for DiffConfig {
    fn default() -> Self {
        Self { context_lines: 3 }
    }
}

impl Default for PickConfig {
    fn default() -> Self {
        Self { preview_lines: 100 }
    }
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_untracked_file_size_mb: 10,
        }
    }
}

// ── Partial config types (Option<T> for field-level merge) ──────────

#[derive(Debug, Deserialize, Default)]
struct PartialConfig {
    global: Option<PartialGlobalConfig>,
    diff: Option<PartialDiffConfig>,
    pick: Option<PartialPickConfig>,
    limits: Option<PartialLimitsConfig>,
}

#[derive(Debug, Deserialize, Default)]
struct PartialGlobalConfig {
    no_copy: Option<bool>,
    no_color: Option<bool>,
    depth: Option<usize>,
    mode: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct PartialDiffConfig {
    context_lines: Option<u32>,
}

#[derive(Debug, Deserialize, Default)]
struct PartialPickConfig {
    preview_lines: Option<usize>,
}

#[derive(Debug, Deserialize, Default)]
struct PartialLimitsConfig {
    max_untracked_file_size_mb: Option<u64>,
}

// ── Merge helpers ───────────────────────────────────────────────────

impl PartialGlobalConfig {
    fn merge(self, overlay: Self) -> Self {
        Self {
            no_copy: overlay.no_copy.or(self.no_copy),
            no_color: overlay.no_color.or(self.no_color),
            depth: overlay.depth.or(self.depth),
            mode: overlay.mode.or(self.mode),
        }
    }
}

impl PartialDiffConfig {
    fn merge(self, overlay: Self) -> Self {
        Self {
            context_lines: overlay.context_lines.or(self.context_lines),
        }
    }
}

impl PartialPickConfig {
    fn merge(self, overlay: Self) -> Self {
        Self {
            preview_lines: overlay.preview_lines.or(self.preview_lines),
        }
    }
}

impl PartialLimitsConfig {
    fn merge(self, overlay: Self) -> Self {
        Self {
            max_untracked_file_size_mb: overlay
                .max_untracked_file_size_mb
                .or(self.max_untracked_file_size_mb),
        }
    }
}

/// Merge two `Option<Section>` values: if both exist, field-merge them;
/// otherwise take whichever is `Some`.
fn merge_section<T: Default>(base: Option<T>, overlay: Option<T>, merge_fn: fn(T, T) -> T) -> Option<T> {
    match (base, overlay) {
        (Some(b), Some(o)) => Some(merge_fn(b, o)),
        (None, o @ Some(_)) => o,
        (b @ Some(_), None) => b,
        (None, None) => None,
    }
}

impl PartialConfig {
    fn merge(self, overlay: Self) -> Self {
        Self {
            global: merge_section(self.global, overlay.global, PartialGlobalConfig::merge),
            diff: merge_section(self.diff, overlay.diff, PartialDiffConfig::merge),
            pick: merge_section(self.pick, overlay.pick, PartialPickConfig::merge),
            limits: merge_section(self.limits, overlay.limits, PartialLimitsConfig::merge),
        }
    }

    fn resolve(self) -> Config {
        let gd = GlobalConfig::default();
        let dd = DiffConfig::default();
        let pd = PickConfig::default();
        let ld = LimitsConfig::default();

        let g = self.global.unwrap_or_default();
        let d = self.diff.unwrap_or_default();
        let p = self.pick.unwrap_or_default();
        let l = self.limits.unwrap_or_default();

        Config {
            global: GlobalConfig {
                no_copy: g.no_copy.unwrap_or(gd.no_copy),
                no_color: g.no_color.unwrap_or(gd.no_color),
                depth: g.depth.unwrap_or(gd.depth),
                mode: g.mode.unwrap_or(gd.mode),
            },
            diff: DiffConfig {
                context_lines: d.context_lines.unwrap_or(dd.context_lines),
            },
            pick: PickConfig {
                preview_lines: p.preview_lines.unwrap_or(pd.preview_lines),
            },
            limits: LimitsConfig {
                max_untracked_file_size_mb: l
                    .max_untracked_file_size_mb
                    .unwrap_or(ld.max_untracked_file_size_mb),
            },
        }
    }
}

// ── Config loading ──────────────────────────────────────────────────

impl Config {
    pub fn load() -> Self {
        let global = Self::load_partial(Self::global_config_path());
        let local = Self::load_partial(Self::local_config_path());
        global.merge(local).resolve()
    }

    fn load_partial(path: Option<PathBuf>) -> PartialConfig {
        let Some(path) = path else {
            return PartialConfig::default();
        };

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return PartialConfig::default(),
            Err(e) => {
                eprintln!("warning: could not read {}: {}", path.display(), e);
                return PartialConfig::default();
            }
        };

        match toml::from_str(&content) {
            Ok(config) => config,
            Err(e) => {
                eprintln!("warning: invalid config at {}: {}", path.display(), e);
                PartialConfig::default()
            }
        }
    }

    fn global_config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("supp").join("supp.toml"))
    }

    fn local_config_path() -> Option<PathBuf> {
        let repo = gix::discover(".").ok()?;
        let workdir = repo.workdir()?.to_path_buf();
        let path = workdir.join("supp.toml");
        path.exists().then_some(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_toml_gives_defaults() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.global.depth, 2);
        assert!(!config.global.no_copy);
        assert!(!config.global.no_color);
        assert_eq!(config.global.mode, "full");
        assert_eq!(config.diff.context_lines, 3);
        assert_eq!(config.pick.preview_lines, 100);
        assert_eq!(config.limits.max_untracked_file_size_mb, 10);
    }

    #[test]
    fn partial_toml_fills_defaults() {
        let config: Config = toml::from_str(
            r#"
[global]
depth = 5
no_copy = true
"#,
        )
        .unwrap();
        assert_eq!(config.global.depth, 5);
        assert!(config.global.no_copy);
        assert!(!config.global.no_color);
        assert_eq!(config.global.mode, "full");
        assert_eq!(config.diff.context_lines, 3);
    }

    #[test]
    fn full_config_parses() {
        let config: Config = toml::from_str(
            r#"
[global]
no_copy = true
no_color = true
depth = 4
mode = "slim"

[diff]
context_lines = 5

[pick]
preview_lines = 50

[limits]
max_untracked_file_size_mb = 20
"#,
        )
        .unwrap();
        assert!(config.global.no_copy);
        assert!(config.global.no_color);
        assert_eq!(config.global.depth, 4);
        assert_eq!(config.global.mode, "slim");
        assert_eq!(config.diff.context_lines, 5);
        assert_eq!(config.pick.preview_lines, 50);
        assert_eq!(config.limits.max_untracked_file_size_mb, 20);
    }

    #[test]
    fn invalid_toml_returns_err() {
        let result = toml::from_str::<Config>("not valid [[[toml");
        assert!(result.is_err());
    }

    // ── Partial merge tests ─────────────────────────────────────────

    fn parse_partial(s: &str) -> PartialConfig {
        toml::from_str(s).unwrap()
    }

    #[test]
    fn partial_merge_local_overrides_global() {
        let global = parse_partial(
            r#"
[global]
depth = 4

[diff]
context_lines = 5
"#,
        );
        let local = parse_partial(
            r#"
[global]
no_copy = true
"#,
        );
        let config = global.merge(local).resolve();
        assert_eq!(config.global.depth, 4);
        assert!(config.global.no_copy);
        assert_eq!(config.diff.context_lines, 5);
    }

    #[test]
    fn partial_merge_local_wins_on_conflict() {
        let global = parse_partial(
            r#"
[global]
depth = 4
mode = "slim"
"#,
        );
        let local = parse_partial(
            r#"
[global]
depth = 1
mode = "map"
"#,
        );
        let config = global.merge(local).resolve();
        assert_eq!(config.global.depth, 1);
        assert_eq!(config.global.mode, "map");
    }

    #[test]
    fn partial_merge_missing_sections() {
        let global = parse_partial(
            r#"
[diff]
context_lines = 10
"#,
        );
        let local = parse_partial(
            r#"
[pick]
preview_lines = 200
"#,
        );
        let config = global.merge(local).resolve();
        assert_eq!(config.diff.context_lines, 10);
        assert_eq!(config.pick.preview_lines, 200);
        // Sections not in either file get defaults
        assert_eq!(config.global.depth, 2);
        assert_eq!(config.limits.max_untracked_file_size_mb, 10);
    }

    #[test]
    fn resolve_fills_defaults() {
        let config = PartialConfig::default().resolve();
        assert_eq!(config.global.depth, 2);
        assert!(!config.global.no_copy);
        assert!(!config.global.no_color);
        assert_eq!(config.global.mode, "full");
        assert_eq!(config.diff.context_lines, 3);
        assert_eq!(config.pick.preview_lines, 100);
        assert_eq!(config.limits.max_untracked_file_size_mb, 10);
    }

    #[test]
    fn partial_from_toml_only_sets_explicit_fields() {
        let partial: PartialConfig = toml::from_str(
            r#"
[global]
depth = 2
"#,
        )
        .unwrap();
        let g = partial.global.unwrap();
        // depth was explicitly set (even though it matches the default)
        assert_eq!(g.depth, Some(2));
        // other fields were not set
        assert!(g.no_copy.is_none());
        assert!(g.no_color.is_none());
        assert!(g.mode.is_none());
    }
}
