//! Persistent Matcha application settings and shell profile discovery.

use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use atomicwrites::{AllowOverwrite, AtomicFile};
use directories::{BaseDirs, ProjectDirs};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const CURRENT_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_SCROLLBACK_LINES: usize = 50_000;
pub const MAX_SCROLLBACK_LINES: usize = 1_000_000;
pub const DEFAULT_FONT_SIZE: f32 = 14.0;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LocalePreference {
    #[default]
    System,
    English,
    SimplifiedChinese,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemePreference {
    #[default]
    MatchaDark,
    Light,
    System,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct AppearanceConfig {
    pub theme: ThemePreference,
    pub locale: LocalePreference,
}

impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            theme: ThemePreference::MatchaDark,
            locale: LocalePreference::System,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct TerminalConfig {
    pub font_family: String,
    pub font_size: f32,
    pub scrollback_lines: usize,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            font_family: "JetBrains Mono".into(),
            font_size: DEFAULT_FONT_SIZE,
            scrollback_lines: DEFAULT_SCROLLBACK_LINES,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ClipboardConfig {
    pub copy_on_select: bool,
    pub confirm_multiline_paste: bool,
    pub allow_osc52_read: bool,
    pub trusted_osc52_write_profiles: Vec<String>,
}

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self::with_safe_defaults()
    }
}

impl ClipboardConfig {
    #[must_use]
    pub fn with_safe_defaults() -> Self {
        Self {
            copy_on_select: false,
            confirm_multiline_paste: true,
            allow_osc52_read: false,
            trusted_osc52_write_profiles: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShellProfileConfig {
    pub id: String,
    pub name: String,
    pub program: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    pub startup_directory: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct AppConfig {
    pub schema_version: u32,
    pub appearance: AppearanceConfig,
    pub terminal: TerminalConfig,
    pub clipboard: ClipboardConfig,
    pub shell_profiles: Vec<ShellProfileConfig>,
    pub default_shell_profile: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        let shell_profiles = discover_shell_profiles();
        let default_shell_profile = shell_profiles
            .first()
            .map_or_else(String::new, |profile| profile.id.clone());
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            appearance: AppearanceConfig::default(),
            terminal: TerminalConfig::default(),
            clipboard: ClipboardConfig::with_safe_defaults(),
            shell_profiles,
            default_shell_profile,
        }
    }
}

impl AppConfig {
    pub fn normalize(&mut self) {
        self.schema_version = CURRENT_SCHEMA_VERSION;
        self.terminal.font_size = self.terminal.font_size.clamp(8.0, 48.0);
        self.terminal.scrollback_lines = self.terminal.scrollback_lines.min(MAX_SCROLLBACK_LINES);
        if self.shell_profiles.is_empty() {
            self.shell_profiles = discover_shell_profiles();
        }
        let mut ids = HashSet::new();
        for profile in &mut self.shell_profiles {
            if profile.id.trim().is_empty() || !ids.insert(profile.id.clone()) {
                profile.id = new_profile_id();
                ids.insert(profile.id.clone());
            }
        }
        if !self
            .shell_profiles
            .iter()
            .any(|profile| profile.id == self.default_shell_profile)
        {
            self.default_shell_profile = self
                .shell_profiles
                .first()
                .map_or_else(String::new, |profile| profile.id.clone());
        }
    }

    #[must_use]
    pub fn default_shell(&self) -> Option<&ShellProfileConfig> {
        self.shell_profiles
            .iter()
            .find(|profile| profile.id == self.default_shell_profile)
            .or_else(|| self.shell_profiles.first())
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ProfileValidationError {
    #[error("profile name is required")]
    EmptyName,
    #[error("profile program is required")]
    EmptyProgram,
    #[error("profile program does not exist or cannot be found in PATH")]
    ProgramNotFound,
    #[error("profile startup directory does not exist")]
    StartupDirectoryNotFound,
}

#[must_use]
pub fn new_profile_id() -> String {
    Uuid::new_v4().to_string()
}

/// Validates a shell profile against the supplied executable search path.
///
/// # Errors
///
/// Returns the first invalid required field or filesystem location.
pub fn validate_shell_profile(
    profile: &ShellProfileConfig,
    path: Option<&OsString>,
) -> Result<(), ProfileValidationError> {
    if profile.name.trim().is_empty() {
        return Err(ProfileValidationError::EmptyName);
    }
    if profile.program.as_os_str().is_empty() {
        return Err(ProfileValidationError::EmptyProgram);
    }
    let program_found = profile.program.is_file()
        || (profile.program.components().count() == 1
            && profile
                .program
                .to_str()
                .and_then(|program| find_in_path(program, path))
                .is_some());
    if !program_found {
        return Err(ProfileValidationError::ProgramNotFound);
    }
    if profile
        .startup_directory
        .as_ref()
        .is_some_and(|directory| !directory.is_dir())
    {
        return Err(ProfileValidationError::StartupDirectoryNotFound);
    }
    Ok(())
}

#[derive(Debug)]
pub struct ConfigLoad {
    pub config: AppConfig,
    pub warning: Option<String>,
    pub path: PathBuf,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("no platform configuration directory is available")]
    NoConfigDirectory,
    #[error("failed to create configuration directory: {0}")]
    CreateDirectory(#[source] std::io::Error),
    #[error("failed to serialize configuration: {0}")]
    Serialize(#[source] toml::ser::Error),
    #[error("failed to atomically write configuration: {0}")]
    Write(#[source] atomicwrites::Error<std::io::Error>),
}

#[must_use]
pub fn config_path() -> Option<PathBuf> {
    ProjectDirs::from("dev", "Matcha", "Matcha").map(|dirs| dirs.config_dir().join("config.toml"))
}

/// Loads the configuration, backing up malformed content and returning safe defaults.
///
/// # Errors
///
/// Returns an error only when the platform has no configuration directory or
/// the directory cannot be created. Invalid TOML is recovered automatically.
pub fn load() -> Result<ConfigLoad, ConfigError> {
    let path = config_path().ok_or(ConfigError::NoConfigDirectory)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(ConfigError::CreateDirectory)?;
    }
    Ok(load_from_path(&path))
}

#[must_use]
pub fn load_from_path(path: &Path) -> ConfigLoad {
    let parsed = fs::read_to_string(path)
        .map_err(|error| error.to_string())
        .and_then(|source| toml::from_str::<AppConfig>(&source).map_err(|error| error.to_string()));

    match parsed {
        Ok(mut config) => {
            config.normalize();
            ConfigLoad {
                config,
                warning: None,
                path: path.to_owned(),
            }
        }
        Err(error) if path.exists() => {
            let backup = invalid_backup_path(path);
            let backup_note = match fs::rename(path, &backup) {
                Ok(()) => format!("; invalid file moved to {}", backup.display()),
                Err(backup_error) => format!("; backup failed: {backup_error}"),
            };
            ConfigLoad {
                config: AppConfig::default(),
                warning: Some(format!("configuration was reset: {error}{backup_note}")),
                path: path.to_owned(),
            }
        }
        Err(_) => ConfigLoad {
            config: AppConfig::default(),
            warning: None,
            path: path.to_owned(),
        },
    }
}

/// Atomically saves a normalized configuration.
///
/// # Errors
///
/// Returns an error if serialization, directory creation, or the atomic
/// replacement fails.
pub fn save(path: &Path, config: &AppConfig) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(ConfigError::CreateDirectory)?;
    }
    let mut normalized = config.clone();
    normalized.normalize();
    let source = toml::to_string_pretty(&normalized).map_err(ConfigError::Serialize)?;
    AtomicFile::new(path, AllowOverwrite)
        .write(|file| file.write_all(source.as_bytes()))
        .map_err(ConfigError::Write)
}

#[must_use]
pub fn discover_shell_profiles() -> Vec<ShellProfileConfig> {
    let home = BaseDirs::new().map(|dirs| dirs.home_dir().to_owned());
    let login_shell = env::var_os("SHELL");
    discover_shell_profiles_with(
        env::var_os("PATH").as_ref(),
        login_shell.as_ref(),
        home.as_ref(),
    )
}

fn discover_shell_profiles_with(
    path: Option<&OsString>,
    login_shell: Option<&OsString>,
    home: Option<&PathBuf>,
) -> Vec<ShellProfileConfig> {
    #[cfg(windows)]
    let _ = &login_shell;

    #[cfg(windows)]
    let candidates = [
        ("powershell-7", "PowerShell 7", "pwsh.exe", vec!["-NoLogo"]),
        (
            "windows-powershell",
            "Windows PowerShell",
            "powershell.exe",
            vec!["-NoLogo"],
        ),
        ("command-prompt", "Command Prompt", "cmd.exe", vec!["/D"]),
    ];

    #[cfg(windows)]
    return candidates
        .into_iter()
        .filter_map(|(id, name, program, args)| {
            find_in_path(program, path).map(|program| ShellProfileConfig {
                id: id.into(),
                name: name.into(),
                program,
                args: args.into_iter().map(String::from).collect(),
                startup_directory: home.cloned(),
            })
        })
        .collect();

    #[cfg(not(windows))]
    {
        let program = login_shell
            .map(PathBuf::from)
            .filter(|shell| shell.is_file())
            .or_else(|| find_in_path("bash", path))
            .unwrap_or_else(|| PathBuf::from("/bin/sh"));
        let name = program
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Shell")
            .to_owned();
        vec![ShellProfileConfig {
            id: "login-shell".into(),
            name,
            program,
            args: Vec::new(),
            startup_directory: home.cloned(),
        }]
    }
}

fn find_in_path(program: &str, path: Option<&OsString>) -> Option<PathBuf> {
    env::split_paths(path?)
        .map(|directory| directory.join(program))
        .find(|candidate| candidate.is_file())
}

fn invalid_backup_path(path: &Path) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    path.with_extension(format!("invalid-{timestamp}.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        env::temp_dir().join(format!("matcha-config-{name}-{}", std::process::id()))
    }

    #[test]
    fn defaults_are_safe_and_normalized() {
        let config = AppConfig::default();
        assert_eq!(config.schema_version, CURRENT_SCHEMA_VERSION);
        assert!((config.terminal.font_size - 14.0).abs() < f32::EPSILON);
        assert_eq!(config.terminal.scrollback_lines, 50_000);
        assert!(!config.clipboard.copy_on_select);
        assert!(config.clipboard.confirm_multiline_paste);
        assert!(!config.clipboard.allow_osc52_read);
    }

    #[test]
    fn config_round_trips_through_toml() {
        let mut config = AppConfig::default();
        config.appearance.locale = LocalePreference::SimplifiedChinese;
        let source = toml::to_string(&config).expect("config should serialize");
        let decoded: AppConfig = toml::from_str(&source).expect("config should deserialize");
        assert_eq!(decoded, config);
    }

    #[test]
    fn omitted_clipboard_fields_keep_safe_defaults() {
        let clipboard: ClipboardConfig =
            toml::from_str("copy_on_select = false").expect("partial clipboard config parses");
        assert!(clipboard.confirm_multiline_paste);
        assert!(!clipboard.allow_osc52_read);
        assert!(clipboard.trusted_osc52_write_profiles.is_empty());
    }

    #[test]
    fn validates_profile_required_fields_and_locations() {
        let executable = env::current_exe().expect("test executable path is available");
        let mut profile = ShellProfileConfig {
            id: new_profile_id(),
            name: "Test shell".into(),
            program: executable,
            args: Vec::new(),
            startup_directory: Some(env::temp_dir()),
        };
        assert_eq!(validate_shell_profile(&profile, None), Ok(()));
        profile.name.clear();
        assert_eq!(
            validate_shell_profile(&profile, None),
            Err(ProfileValidationError::EmptyName)
        );
    }

    #[test]
    fn malformed_config_is_backed_up_and_recovered() {
        let directory = temp_path("malformed");
        fs::create_dir_all(&directory).expect("temporary directory should be created");
        let path = directory.join("config.toml");
        fs::write(&path, "not = [valid").expect("invalid config should be written");

        let loaded = load_from_path(&path);
        assert!(loaded.warning.is_some());
        assert!(!path.exists());
        assert_eq!(loaded.config.schema_version, CURRENT_SCHEMA_VERSION);

        fs::remove_dir_all(directory).expect("temporary directory should be removed");
    }

    #[test]
    fn save_is_readable_and_clamps_values() {
        let directory = temp_path("save");
        let path = directory.join("config.toml");
        let mut config = AppConfig::default();
        config.terminal.font_size = 200.0;
        config.terminal.scrollback_lines = usize::MAX;
        save(&path, &config).expect("config should save");

        let loaded = load_from_path(&path);
        assert!((loaded.config.terminal.font_size - 48.0).abs() < f32::EPSILON);
        assert_eq!(
            loaded.config.terminal.scrollback_lines,
            MAX_SCROLLBACK_LINES
        );

        fs::remove_dir_all(directory).expect("temporary directory should be removed");
    }
}
