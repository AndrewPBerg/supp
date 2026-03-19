use serde::Deserialize;
use std::path::PathBuf;

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

impl Config {
    pub fn load() -> Self {
        let Some(path) = Self::config_path() else {
            return Self::default();
        };

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                eprintln!("warning: could not read {}: {}", path.display(), e);
                return Self::default();
            }
        };

        match toml::from_str(&content) {
            Ok(config) => config,
            Err(e) => {
                eprintln!("warning: invalid config at {}: {}", path.display(), e);
                Self::default()
            }
        }
    }

    fn config_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".supp").join("config.toml"))
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
}
