// ── Public config types ──────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct Config {
    pub global: GlobalConfig,
    pub diff: DiffConfig,
    pub pick: PickConfig,
    pub limits: LimitsConfig,
}

#[derive(Debug)]
pub struct GlobalConfig {
    pub no_copy: bool,
    pub no_color: bool,
    pub json: bool,
    pub depth: usize,
    pub mode: String,
}

#[derive(Debug)]
pub struct DiffConfig {
    pub context_lines: u32,
}

#[derive(Debug)]
pub struct PickConfig {
    pub preview_lines: usize,
}

#[derive(Debug)]
pub struct LimitsConfig {
    pub max_untracked_file_size_mb: u64,
    pub max_files: usize,
    pub max_total_mb: u64,
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            no_copy: false,
            no_color: false,
            json: false,
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
            max_files: 20000,
            max_total_mb: 50,
        }
    }
}

impl Config {
    pub fn load() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults() {
        let config = Config::default();
        assert!(!config.global.no_copy);
        assert!(!config.global.no_color);
        assert!(!config.global.json);
        assert_eq!(config.global.depth, 2);
        assert_eq!(config.global.mode, "full");
        assert_eq!(config.diff.context_lines, 3);
        assert_eq!(config.pick.preview_lines, 100);
        assert_eq!(config.limits.max_untracked_file_size_mb, 10);
    }

    #[test]
    fn config_load_returns_defaults() {
        let config = Config::load();
        assert_eq!(config.global.depth, 2);
        assert_eq!(config.global.mode, "full");
    }

    #[test]
    fn diff_config_defaults() {
        let config = DiffConfig::default();
        assert_eq!(config.context_lines, 3);
    }

    #[test]
    fn pick_config_defaults() {
        let config = PickConfig::default();
        assert_eq!(config.preview_lines, 100);
    }

    #[test]
    fn limits_config_defaults() {
        let config = LimitsConfig::default();
        assert_eq!(config.max_untracked_file_size_mb, 10);
        assert_eq!(config.max_files, 20000);
        assert_eq!(config.max_total_mb, 50);
    }

    #[test]
    fn global_config_debug() {
        let config = GlobalConfig::default();
        let debug = format!("{:?}", config);
        assert!(debug.contains("GlobalConfig"));
    }

    #[test]
    fn config_debug() {
        let config = Config::default();
        let debug = format!("{:?}", config);
        assert!(debug.contains("Config"));
    }
}
