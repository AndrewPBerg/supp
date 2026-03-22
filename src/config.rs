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

// ── Performance modes ───────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerfMode {
    Full,
    Balanced,
    Lite,
}

#[derive(Debug, Clone)]
pub struct PerfProfile {
    pub rayon_threads: usize,
    pub pagerank_iters: usize,
    pub max_files: usize,
    pub max_total_mb: u64,
    pub call_sites_cap: usize,
    pub call_sites_early_exit: bool,
    pub used_by_file_threshold: usize,
}

impl PerfMode {
    pub fn profile(&self) -> PerfProfile {
        match self {
            PerfMode::Full => PerfProfile {
                rayon_threads: 0, // all cores (rayon default)
                pagerank_iters: 15,
                max_files: 20_000,
                max_total_mb: 50,
                call_sites_cap: 30,
                call_sites_early_exit: false,
                used_by_file_threshold: 20,
            },
            PerfMode::Balanced => PerfProfile {
                rayon_threads: std::thread::available_parallelism()
                    .map(|n| (n.get() / 2).max(2))
                    .unwrap_or(2),
                pagerank_iters: 8,
                max_files: 50_000,
                max_total_mb: 100,
                call_sites_cap: 30,
                call_sites_early_exit: true,
                used_by_file_threshold: 10,
            },
            PerfMode::Lite => PerfProfile {
                rayon_threads: 2,
                pagerank_iters: 5,
                max_files: 10_000,
                max_total_mb: 30,
                call_sites_cap: 15,
                call_sites_early_exit: true,
                used_by_file_threshold: 0,
            },
        }
    }
}

impl std::fmt::Display for PerfMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PerfMode::Full => write!(f, "full"),
            PerfMode::Balanced => write!(f, "balanced"),
            PerfMode::Lite => write!(f, "lite"),
        }
    }
}

impl std::str::FromStr for PerfMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "full" => Ok(PerfMode::Full),
            "balanced" => Ok(PerfMode::Balanced),
            "lite" => Ok(PerfMode::Lite),
            _ => Err(format!(
                "unknown perf mode '{}': expected 'full', 'balanced', or 'lite'",
                s
            )),
        }
    }
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
        }
    }
}

impl Config {
    pub fn load() -> Self {
        Self::default()
    }
}

// ── Perf mode persistence ───────────────────────────────────────────

use std::path::PathBuf;

fn config_dir() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|_| std::env::var("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|_| PathBuf::from(".config"))
        .join("supp")
}

pub fn perf_config_path() -> PathBuf {
    config_dir().join("perf")
}

pub fn load_perf_mode() -> PerfMode {
    std::fs::read_to_string(perf_config_path())
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(PerfMode::Full)
}

pub fn save_perf_mode(mode: PerfMode) -> anyhow::Result<()> {
    let path = perf_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, format!("{}\n", mode))?;
    Ok(())
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

    // ── PerfMode tests ──────────────────────────────────────────────

    #[test]
    fn perf_mode_from_str() {
        assert_eq!("full".parse::<PerfMode>().unwrap(), PerfMode::Full);
        assert_eq!("balanced".parse::<PerfMode>().unwrap(), PerfMode::Balanced);
        assert_eq!("lite".parse::<PerfMode>().unwrap(), PerfMode::Lite);
        assert_eq!("FULL".parse::<PerfMode>().unwrap(), PerfMode::Full);
        assert_eq!("Balanced".parse::<PerfMode>().unwrap(), PerfMode::Balanced);
        assert!("unknown".parse::<PerfMode>().is_err());
    }

    #[test]
    fn perf_mode_display() {
        assert_eq!(PerfMode::Full.to_string(), "full");
        assert_eq!(PerfMode::Balanced.to_string(), "balanced");
        assert_eq!(PerfMode::Lite.to_string(), "lite");
    }

    #[test]
    fn perf_profile_full() {
        let p = PerfMode::Full.profile();
        assert_eq!(p.rayon_threads, 0);
        assert_eq!(p.pagerank_iters, 15);
        assert_eq!(p.max_files, 20_000);
        assert_eq!(p.max_total_mb, 50);
        assert_eq!(p.call_sites_cap, 30);
        assert!(!p.call_sites_early_exit);
        assert_eq!(p.used_by_file_threshold, 20);
    }

    #[test]
    fn perf_profile_balanced() {
        let p = PerfMode::Balanced.profile();
        assert!(p.rayon_threads >= 2);
        assert_eq!(p.pagerank_iters, 8);
        assert_eq!(p.max_files, 50_000);
        assert_eq!(p.max_total_mb, 100);
        assert_eq!(p.call_sites_cap, 30);
        assert!(p.call_sites_early_exit);
        assert_eq!(p.used_by_file_threshold, 10);
    }

    #[test]
    fn perf_profile_lite() {
        let p = PerfMode::Lite.profile();
        assert_eq!(p.rayon_threads, 2);
        assert_eq!(p.pagerank_iters, 5);
        assert_eq!(p.max_files, 10_000);
        assert_eq!(p.max_total_mb, 30);
        assert_eq!(p.call_sites_cap, 15);
        assert!(p.call_sites_early_exit);
        assert_eq!(p.used_by_file_threshold, 0);
    }
}
