use figment::providers::Format;
use figment::{
	Figment,
	providers::{Env, Toml},
};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

#[derive(Debug, Deserialize, Serialize)]
pub struct Settings {
	#[serde(default = "default_data_path")]
	pub data_path: String,
	#[serde(default)]
	pub discord_token: String,
	#[serde(default)]
	pub network: Network,
	#[serde(default)]
	pub catchup: Catchup,
	#[serde(default)]
	pub metadata: Metadata,
	#[serde(default)]
	pub storage: Storage,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Catchup {
	/// Number of messages to fetch per Discord API request.
	#[serde(default = "default_messages_per_request")]
	pub messages_per_request: u16,

	/// Number of messages to batch before writing to storage.
	#[serde(default = "default_write_batch_size")]
	pub write_batch_size: usize,

	/// Maximum number of concurrent channel catchups.
	#[serde(default = "default_channel_concurrency")]
	pub channel_concurrency: usize,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Metadata {
	/// Number of members to fetch per Discord API request during sync.
	#[serde(default = "default_member_fetch_limit")]
	pub member_fetch_limit: u16,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Storage {
	/// How often to flush the log buffer, in milliseconds.
	#[serde(default = "default_autoflush_interval_ms")]
	pub autoflush_interval_ms: u64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Network {
	/// Timeout for network requests in seconds.
	#[serde(default = "default_network_timeout")]
	pub timeout: u64,

	/// The number of concurrent asset downloads allowed.
	#[serde(default = "default_download_concurrency")]
	pub download_concurrency_limit: usize,
}

fn default_data_path() -> String {
	"./data".to_string()
}

const fn default_network_timeout() -> u64 {
	120
}

const fn default_download_concurrency() -> usize {
	10
}

const fn default_messages_per_request() -> u16 {
	100
}

const fn default_write_batch_size() -> usize {
	1000
}

const fn default_channel_concurrency() -> usize {
	4
}

const fn default_member_fetch_limit() -> u16 {
	1000
}

const fn default_autoflush_interval_ms() -> u64 {
	60000 // 1m
}

impl Default for Storage {
	fn default() -> Self {
		Self {
			autoflush_interval_ms: default_autoflush_interval_ms(),
		}
	}
}

impl Default for Catchup {
	fn default() -> Self {
		Self {
			messages_per_request: default_messages_per_request(),
			write_batch_size: default_write_batch_size(),
			channel_concurrency: default_channel_concurrency(),
		}
	}
}

impl Default for Metadata {
	fn default() -> Self {
		Self {
			member_fetch_limit: default_member_fetch_limit(),
		}
	}
}

impl Default for Network {
	fn default() -> Self {
		Self {
			timeout: default_network_timeout(),
			download_concurrency_limit: default_download_concurrency(),
		}
	}
}

impl Default for Settings {
	fn default() -> Self {
		Self {
			data_path: default_data_path(),
			discord_token: String::new(),
			network: Network::default(),
			catchup: Catchup::default(),
			metadata: Metadata::default(),
			storage: Storage::default(),
		}
	}
}

pub static SETTINGS: LazyLock<Settings> = LazyLock::new(Settings::load);

impl Settings {
	pub fn load() -> Self {
		let config_path = "config.toml";

		if !Path::new(config_path).exists() {
			create_default_config_file(config_path);
		}

		Figment::new()
			.merge(Toml::file(config_path))
			.merge(Env::prefixed("BIGBROTHER_"))
			.extract()
			.expect("Failed to load configuration")
	}
}

fn create_default_config_file(path: &str) {
	let default_settings = Settings::default();
	let toml_string = toml::to_string_pretty(&default_settings).expect("Failed to serialize default settings");

	fs::write(path, toml_string).expect("Failed to write default settings file. Check permissions.");

	println!("Created default configuration file at '{path}'.");
	println!("Set the BIGBROTHER_DISCORD_TOKEN environment variable or \"discord_token\" in config.toml, then restart the application.");
	std::process::exit(1);
}
