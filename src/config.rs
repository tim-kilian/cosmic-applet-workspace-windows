// SPDX-License-Identifier: GPL-3.0-only

use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

const CONFIG_DIR_NAME: &str = "io.github.tkilian.CosmicAppletAppTitle";
const CONFIG_FILE_NAME: &str = "config.toml";

pub const DEFAULT_TITLE_CHARS: usize = 24;
pub const MIN_TITLE_CHARS: usize = 8;
pub const MAX_TITLE_CHARS: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppletConfig {
    pub max_title_chars: usize,
    pub middle_click_closes: bool,
    pub show_app_icons: bool,
    pub show_hover_close_button: bool,
}

impl Default for AppletConfig {
    fn default() -> Self {
        Self {
            max_title_chars: DEFAULT_TITLE_CHARS,
            middle_click_closes: true,
            show_app_icons: true,
            show_hover_close_button: true,
        }
    }
}

impl AppletConfig {
    pub fn load() -> Self {
        let Some(path) = config_path() else {
            return Self::default();
        };

        match fs::read_to_string(&path) {
            Ok(contents) => match toml::from_str::<Self>(&contents) {
                Ok(config) => config.normalized(),
                Err(err) => {
                    tracing::warn!("Failed to parse config at {}: {err}", path.display());
                    Self::default()
                }
            },
            Err(err) if err.kind() == ErrorKind::NotFound => Self::default(),
            Err(err) => {
                tracing::warn!("Failed to read config at {}: {err}", path.display());
                Self::default()
            }
        }
    }

    pub fn save(&self) {
        let Some(path) = config_path() else {
            return;
        };

        let Some(parent) = path.parent() else {
            return;
        };

        if let Err(err) = fs::create_dir_all(parent) {
            tracing::warn!(
                "Failed to create config directory {}: {err}",
                parent.display()
            );
            return;
        }

        let normalized = self.clone().normalized();
        let contents = match toml::to_string_pretty(&normalized) {
            Ok(contents) => contents,
            Err(err) => {
                tracing::warn!("Failed to serialize config: {err}");
                return;
            }
        };

        if let Err(err) = fs::write(&path, contents) {
            tracing::warn!("Failed to write config at {}: {err}", path.display());
        }
    }

    pub fn normalized(mut self) -> Self {
        self.max_title_chars = self.max_title_chars.clamp(MIN_TITLE_CHARS, MAX_TITLE_CHARS);
        self
    }
}

fn config_path() -> Option<PathBuf> {
    let mut path = dirs::config_dir()?;
    path.push(Path::new(CONFIG_DIR_NAME));
    path.push(Path::new(CONFIG_FILE_NAME));
    Some(path)
}
