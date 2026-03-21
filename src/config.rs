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
        Self::default()
    }
}
