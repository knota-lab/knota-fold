use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppLogsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_db_path")]
    pub db_path: String,
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    #[serde(default = "default_batch_interval")]
    pub batch_interval_ms: u64,
    #[serde(default = "default_retention")]
    pub retention_days: u32,
    #[serde(default = "default_capture_level")]
    pub capture_level: String,
    #[serde(default = "default_true")]
    pub capture_log_entries: bool,
    #[serde(default = "default_max_body")]
    pub max_body_size: usize,
}

fn default_db_path() -> String {
    "data/logs/app.db".to_string()
}
fn default_batch_size() -> usize {
    50
}
fn default_batch_interval() -> u64 {
    100
}
fn default_retention() -> u32 {
    7
}
fn default_capture_level() -> String {
    "warn".to_string()
}
fn default_true() -> bool {
    true
}
fn default_max_body() -> usize {
    4096
}

impl AppLogsConfig {
    /// Read from `settings.appLogs` section of loco config.
    pub fn from_settings(settings: &serde_json::Value) -> Option<Self> {
        settings
            .get("appLogs")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    /// Convert capture_level string to tracing LevelFilter.
    pub fn level_filter(&self) -> tracing::Level {
        match self.capture_level.as_str() {
            "debug" => tracing::Level::DEBUG,
            "info" => tracing::Level::INFO,
            "warn" => tracing::Level::WARN,
            "error" => tracing::Level::ERROR,
            _ => tracing::Level::WARN,
        }
    }
}
