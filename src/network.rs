use crate::settings::SETTINGS;
use crate::storage::LogStore;
use crate::utils::ensure_dir;
use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest::Client;
use reqwest::header::CONTENT_LENGTH;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;
use tracing::{error, info, instrument, trace, warn};

static CLIENT: LazyLock<Client> = LazyLock::new(|| {
	Client::builder()
		.hickory_dns(true)
		.https_only(true)
		.http3_prior_knowledge()
		.timeout(Duration::from_secs(SETTINGS.network.timeout))
		.build()
		.expect("Failed to create reqwest client")
});

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct DownloadRequest {
	pub url: String,
	pub folder: PathBuf,
	pub filename: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "t")]
enum DownloadLogEvent {
	#[serde(rename = "s")]
	Start(DownloadRequest),
	#[serde(rename = "c")]
	Complete(DownloadRequest),
}

#[derive(Debug)]
pub struct DownloadTracker {
	log_store: LogStore,
}

impl DownloadTracker {
	pub fn new(shutdown: &Arc<AtomicBool>) -> Result<Self> {
		let path = Path::new(&SETTINGS.data_path).join("downloads.ndjson");
		let log_store = LogStore::new(path, shutdown)?;
		Ok(Self { log_store })
	}

	pub fn log_start(&self, req: &DownloadRequest) -> Result<()> {
		let event = DownloadLogEvent::Start(req.clone());
		self.log_store.append(&event)
	}

	pub fn log_complete(&self, req: &DownloadRequest) -> Result<()> {
		let event = DownloadLogEvent::Complete(req.clone());
		self.log_store.append(&event)
	}

	pub async fn get_pending_downloads(&self) -> Result<Vec<DownloadRequest>> {
		let events = match self.log_store.read_all::<DownloadLogEvent>().await {
			Ok(events) => events,
			Err(e) => {
				error!("Failed to read download log, cannot resume downloads: {}", e);
				return Ok(Vec::new());
			}
		};

		let mut states = HashMap::new();

		for event in events {
			match event.payload {
				DownloadLogEvent::Start(req) => {
					states.insert(req, false);
				}
				DownloadLogEvent::Complete(req) => {
					states.insert(req, true);
				}
			}
		}

		let pending = states
			.into_iter()
			.filter_map(|(req, is_completed)| if is_completed { None } else { Some(req) })
			.collect();

		Ok(pending)
	}

	pub async fn clear_log(&self) -> Result<()> {
		trace!("Pending downloads reached 0. Clearing download log.");
		self.log_store.clear().await
	}
}

/// A long-running task that orchestrates file downloads.
#[instrument(skip_all)]
pub async fn asset_downloader_worker(
	mut rx: mpsc::Receiver<DownloadRequest>,
	pending_count: Arc<AtomicUsize>,
	tracker: Arc<DownloadTracker>,
	shutdown: Arc<AtomicBool>,
) {
	info!("Asset downloader orchestration started.");

	let semaphore = Arc::new(Semaphore::new(SETTINGS.network.download_concurrency_limit));

	let mut join_set = JoinSet::new();

	loop {
		tokio::select! {
			_ = join_set.join_next(), if !join_set.is_empty() => {}

			received = rx.recv() => {
				if let Some(req) = received {
					if shutdown.load(Ordering::Relaxed) {
						break;
					}

					let Ok(permit) = semaphore.clone().acquire_owned().await else { break };

					let count = pending_count.clone();
					let track = tracker.clone();
					let sd = shutdown.clone();

					join_set.spawn(async move {
						let _permit = permit;

						process_download(req, count, track, sd).await;
					});
				} else {
					info!("Asset channel closed.");
					break;
				}
			}
		}
	}

	while join_set.join_next().await.is_some() {}

	info!("Asset downloader worker finished.");
}

async fn process_download(req: DownloadRequest, counter: Arc<AtomicUsize>, tracker: Arc<DownloadTracker>, shutdown: Arc<AtomicBool>) {
	if shutdown.load(Ordering::Relaxed) {
		counter.fetch_sub(1, Ordering::SeqCst);
		return;
	}

	if let Err(e) = ensure_dir(&req.folder).await {
		error!(?req.folder, error = %e, "Failed to create directory");
		counter.fetch_sub(1, Ordering::SeqCst);
		return;
	}

	let download_result = download_file(&req.url, &req.folder, &req.filename).await;

	if let Err(e) = &download_result {
		warn!(
			filename = %req.filename,
			url = %req.url,
			error = ?e,
			"Download failed. Will be retried on next launch."
		);
	} else if let Err(e) = tracker.log_complete(&req) {
		error!(?req, error = %e, "Failed to log download completion");
	}

	let previous_count = counter.fetch_sub(1, Ordering::SeqCst);

	if previous_count == 1
		&& let Err(e) = tracker.clear_log().await
	{
		error!("Failed to clear download log: {}", e);
	}
}

#[instrument(skip(output_dir), fields(filename = filename, url = url))]
pub async fn download_file(url: &str, output_dir: &Path, filename: &str) -> Result<()> {
	let final_path = output_dir.join(filename);

	// Skip if already exists
	if let Ok(meta) = tokio::fs::metadata(&final_path).await
		&& meta.len() > 0
	{
		return Ok(());
	}

	let response = CLIENT
		.get(url)
		.send()
		.await
		.context(format!("Failed to send request for URL: {url}"))?;

	if !response.status().is_success() {
		return Err(anyhow::anyhow!("Request failed with status code: {}", response.status()));
	}

	let content_length = response
		.headers()
		.get(CONTENT_LENGTH)
		.and_then(|v| v.to_str().ok())
		.and_then(|v| v.parse::<u64>().ok());

	// Write to a .part file and then rename. Otherwise, a corrupted file from a crash will be skipped by the file existence check.
	let temp_filename = format!("{filename}.part");
	let temp_path = output_dir.join(&temp_filename);

	let file = File::create(&temp_path)
		.await
		.context(format!("Failed to create temp file: {}", temp_path.display()))?;

	if let Some(len) = content_length
		&& let Err(e) = file.set_len(len).await
	{
		warn!("Failed to pre-allocate file size for {url}: {e}");
	}

	let mut writer = BufWriter::with_capacity(64 * 1024, file);
	let mut stream = response.bytes_stream();

	while let Some(chunk) = stream.next().await {
		let chunk = chunk.context(format!("Error reading chunk from URL: {url}"))?;
		writer
			.write_all(&chunk)
			.await
			.context(format!("Error writing to file: {}", temp_path.display()))?;
	}

	writer
		.flush()
		.await
		.context(format!("Error flushing file: {}", temp_path.display()))?;
	drop(writer);

	tokio::fs::rename(&temp_path, &final_path)
		.await
		.context("Failed to rename temp file to final path")?;

	trace!("Successfully downloaded file.");
	Ok(())
}

static CDN_URL: &str = "https://cdn.discordapp.com/";
pub fn append_cdn(to_append: &[&str]) -> String {
	let mut result = String::with_capacity(CDN_URL.len() + 35); // 35 accounts for the length of a Discord ID (20 chars) and some extra
	result.push_str(CDN_URL);
	for part in to_append {
		result.push_str(part);
	}
	result
}
