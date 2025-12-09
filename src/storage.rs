use crate::settings::SETTINGS;
use crate::utils::get_current_time_millis;
use anyhow::{Context, Result};
use serde::{Serialize, de::DeserializeOwned};
use std::fs::File as StdFile;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, error};

#[derive(serde::Deserialize, serde::Serialize, Debug, Clone)]
pub struct LogEvent<T> {
	#[serde(rename = "ts")]
	pub timestamp: u64,
	#[serde(flatten)]
	pub payload: T,
}

enum StoreCommand {
	Write(Vec<u8>),
	Flush(oneshot::Sender<()>),
}

#[derive(Debug, Clone)]
pub struct LogStore {
	path: PathBuf,
	tx: mpsc::UnboundedSender<StoreCommand>,
}

impl LogStore {
	pub fn new(path: PathBuf, shutdown: &Arc<AtomicBool>) -> Result<Self> {
		let (tx, mut rx) = mpsc::unbounded_channel();
		let path_clone = path.clone();
		let shutdown_for_writer = shutdown.clone();

		if let Some(parent) = path.parent() {
			fs::create_dir_all(parent).with_context(|| format!("Failed to create directory: {}", parent.display()))?;
		}

		tokio::task::spawn_blocking(move || {
			let file = match OpenOptions::new().create(true).append(true).open(&path_clone) {
				Ok(f) => f,
				Err(e) => {
					error!("FATAL: LogStore writer failed to open file {:?}: {}", path_clone, e);
					return;
				}
			};

			let mut writer = BufWriter::with_capacity(64 * 1024, file);

			let mut scratchpad = Vec::with_capacity(8 * 1024);

			loop {
				if shutdown_for_writer.load(Ordering::Relaxed) {
					while let Ok(cmd) = rx.try_recv() {
						if let StoreCommand::Write(b) = cmd {
							let _ = writer.write_all(&b);
							let _ = writer.write_all(b"\n");
						}
					}
					let _ = writer.flush();
					break;
				}

				let Some(cmd) = rx.blocking_recv() else { break };

				match cmd {
					StoreCommand::Write(bytes) => {
						scratchpad.clear();
						scratchpad.extend_from_slice(&bytes);
						scratchpad.push(b'\n');

						let mut count = 0;
						while count < 500 && scratchpad.len() < 1024 * 1024 {
							match rx.try_recv() {
								Ok(StoreCommand::Write(b)) => {
									scratchpad.extend_from_slice(&b);
									scratchpad.push(b'\n');
									count += 1;
								}
								Ok(StoreCommand::Flush(tx)) => {
									if let Err(e) = writer.write_all(&scratchpad) {
										error!("Failed to write to log: {}", e);
									}
									if let Err(e) = writer.flush() {
										error!("Failed to flush log: {}", e);
									}
									let _ = tx.send(());
									scratchpad.clear();
									break;
								}
								Err(_) => break,
							}
						}

						if !scratchpad.is_empty()
							&& let Err(e) = writer.write_all(&scratchpad)
						{
							error!("Failed to write to log: {}", e);
						}
					}
					StoreCommand::Flush(respond_to) => {
						if let Err(e) = writer.flush() {
							error!("Failed to flush log: {}", e);
						}
						let _ = respond_to.send(());
					}
				}
			}
			debug!("LogStore writer for {:?} shutting down.", path_clone);
		});

		let tx_flush = tx.clone();
		let shutdown_flush = shutdown.clone();
		tokio::spawn(async move {
			let mut interval = tokio::time::interval(Duration::from_millis(SETTINGS.storage.autoflush_interval_ms));
			interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

			loop {
				interval.tick().await;
				if shutdown_flush.load(Ordering::Relaxed) {
					break;
				}

				let (oneshot_tx, _) = oneshot::channel();
				if tx_flush.send(StoreCommand::Flush(oneshot_tx)).is_err() {
					break;
				}
			}
		});

		Ok(Self { path, tx })
	}

	pub fn append<P: Serialize + Sync + Send + 'static>(&self, payload: &P) -> Result<()> {
		let event = LogEvent {
			timestamp: get_current_time_millis()?,
			payload,
		};
		let json_bytes = sonic_rs::to_vec(&event)?;

		self.tx
			.send(StoreCommand::Write(json_bytes))
			.map_err(|_| anyhow::anyhow!("LogStore writer is closed"))?;
		Ok(())
	}

	pub fn append_bulk<P: Serialize + Sync + Send + 'static>(&self, payloads: Vec<P>) -> Result<()> {
		if payloads.is_empty() {
			return Ok(());
		}
		let ts = get_current_time_millis()?;

		let mut buffer = Vec::new();

		for p in payloads {
			let event = LogEvent { timestamp: ts, payload: p };
			let bytes = sonic_rs::to_vec(&event)?;
			buffer.extend_from_slice(&bytes);
			buffer.push(b'\n');
		}

		if let Some(last) = buffer.last()
			&& *last == b'\n'
		{
			buffer.pop();
		}

		self.tx
			.send(StoreCommand::Write(buffer))
			.map_err(|_| anyhow::anyhow!("LogStore writer is closed"))?;

		Ok(())
	}

	pub async fn read_all<P: DeserializeOwned + Send + 'static>(&self) -> Result<Vec<LogEvent<P>>> {
		let path = self.path.clone();
		tokio::task::spawn_blocking(move || {
			let file = match File::open(&path) {
				Ok(f) => f,
				Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
				Err(e) => return Err(e.into()),
			};

			let mut reader = BufReader::new(file);
			let mut line_buf = String::new();
			let mut events = Vec::new();

			while reader.read_line(&mut line_buf)? > 0 {
				let trimmed = line_buf.trim();
				if !trimmed.is_empty()
					&& let Ok(e) = sonic_rs::from_str::<LogEvent<P>>(trimmed)
				{
					events.push(e);
				}
				line_buf.clear();
			}
			Ok(events)
		})
		.await?
	}

	/// Scans the log file backwards, deserializing entries into `P`.
	pub async fn scan_last<P, R, F>(&self, scanner: F) -> Result<Option<R>>
	where
		P: DeserializeOwned + Send + 'static,
		R: Send + 'static,
		F: Fn(P) -> Option<R> + Send + Sync + 'static,
	{
		self.flush().await?;

		let path = self.path.clone();

		tokio::task::spawn_blocking(move || {
			const CAP: usize = 64 * 1024;

			let mut file = match StdFile::open(&path) {
				Ok(f) => f,
				Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
				Err(e) => return Err(e).context("Failed to open log file for scanning"),
			};

			let file_len = file.metadata()?.len();
			if file_len == 0 {
				return Ok(None);
			}

			let mut buffer = vec![0u8; CAP];

			let mut line_suffix: Vec<u8> = Vec::new();
			let mut file_pos = file_len;

			while file_pos > 0 {
				#[allow(clippy::cast_possible_truncation)]
				let read_len = std::cmp::min(file_pos, CAP as u64) as usize;
				file_pos -= read_len as u64;

				file.seek(SeekFrom::Start(file_pos))?;
				file.read_exact(&mut buffer[..read_len])?;

				let window = &buffer[..read_len];
				let mut cursor = read_len;

				// Scan backwards for newlines within the chunk
				while let Some(newline_idx) = window[..cursor].iter().rposition(|&b| b == b'\n') {
					let line_slice = &window[newline_idx + 1..cursor];

					let bytes_to_parse = if line_suffix.is_empty() {
						line_slice
					} else {
						&[line_slice, &line_suffix].concat()
					};

					if !bytes_to_parse.is_empty()
						&& let Ok(entry) = sonic_rs::from_slice::<P>(bytes_to_parse)
						&& let Some(found) = scanner(entry)
					{
						return Ok(Some(found));
					}

					line_suffix.clear();
					cursor = newline_idx;
				}

				if cursor > 0 {
					let prefix = &window[0..cursor];
					let mut new_suffix = Vec::with_capacity(prefix.len() + line_suffix.len());
					new_suffix.extend_from_slice(prefix);
					new_suffix.append(&mut line_suffix);
					line_suffix = new_suffix;
				}
			}

			if !line_suffix.is_empty()
				&& let Ok(entry) = sonic_rs::from_slice::<P>(&line_suffix)
				&& let Some(found) = scanner(entry)
			{
				return Ok(Some(found));
			}

			Ok(None)
		})
		.await?
	}

	pub async fn flush(&self) -> Result<()> {
		let (tx, rx) = oneshot::channel();
		self.tx
			.send(StoreCommand::Flush(tx))
			.map_err(|_| anyhow::anyhow!("LogStore closed"))?;
		rx.await.context("Flush responder dropped")?;
		Ok(())
	}

	pub async fn clear(&self) -> Result<()> {
		self.flush().await?;

		let path = self.path.clone();
		tokio::task::spawn_blocking(move || OpenOptions::new().write(true).truncate(true).open(path)).await??;

		Ok(())
	}

	pub fn path(&self) -> &Path {
		&self.path
	}
}
