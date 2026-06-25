use anyhow::{Context, bail, ensure};
use async_openai::types::chat::ReasoningEffort;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    pub providers: BTreeMap<String, Provider>,
    pub models: BTreeMap<String, Model>,
}

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let dir =
            directories::ProjectDirs::from("me.caelunshun", "Not Novideo Corporation", "proofbot")
                .context("failed to get dirs")?;

        let path = dir.config_dir().join("config.toml");
        if !path.exists() {
            fs::create_dir_all(dir.config_dir())?;
            fs::write(&path, toml::to_string_pretty(&Self::default())?.as_bytes())?;
            bail!("config was empty, please update it in {}", path.display())
        } else {
            let config: Config = toml::from_str(&fs::read_to_string(&path)?)?;

            for model in config.models.values() {
                ensure!(config.providers.contains_key(&model.provider));
            }

            Ok(config)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub url: String,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    pub provider: String,
    pub api_name: String,
    pub max_output_tokens: u32,
    pub reasoning_level: String,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub top_k: Option<u32>,
}
