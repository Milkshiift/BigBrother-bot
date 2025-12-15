mod catchup;
mod dispatch;
mod error;
mod messages;
mod metadata;
mod network;
mod settings;
mod storage;
mod utils;

use crate::catchup::run_full_guild_catchup;
use crate::error::ProcessorError;
use crate::messages::ChannelArchiver;
use crate::metadata::MetadataArchiver;
use crate::network::{DownloadRequest, DownloadTracker, asset_downloader_worker};
use crate::settings::SETTINGS;
use crate::utils::HumanUptime;
use anyhow::Context;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::sync::mpsc::Sender;
use tracing::{error, info, instrument, warn};
use tracing_appender::non_blocking;
use tracing_subscriber::{EnvFilter, FmtSubscriber};
use twilight_cache_inmemory::{DefaultInMemoryCache, InMemoryCache, ResourceType};
use twilight_gateway::{Event, EventTypeFlags, Intents, Shard, ShardId, StreamExt as _};
use twilight_http::Client as HttpClient;
use twilight_model::gateway::CloseFrame;
use twilight_model::id::Id;
use twilight_model::id::marker::{ChannelMarker, GuildMarker};

#[global_allocator]
static ALLOC: snmalloc_rs::SnMalloc = snmalloc_rs::SnMalloc;

#[derive(Debug)]
pub enum GuildQueueEvent {
	InitialCatchup,
	GatewayEvent(Box<Event>),
}

#[derive(Clone)]
pub struct State {
	pub http: Arc<HttpClient>,
	pub cache: Arc<InMemoryCache>,
	pub file_downloader: Sender<DownloadRequest>,
	pub pending_downloads: Arc<AtomicUsize>,
	pub download_tracker: Arc<DownloadTracker>,
	pub shutdown: Arc<AtomicBool>,
}

impl State {
	pub const fn new(
		http: Arc<HttpClient>,
		cache: Arc<InMemoryCache>,
		file_downloader: Sender<DownloadRequest>,
		pending_downloads: Arc<AtomicUsize>,
		download_tracker: Arc<DownloadTracker>,
		shutdown: Arc<AtomicBool>,
	) -> Self {
		Self {
			http,
			cache,
			file_downloader,
			pending_downloads,
			download_tracker,
			shutdown,
		}
	}

	pub fn submit_download(&self, url: String, folder: PathBuf, filename: String) {
		if self.shutdown.load(Ordering::SeqCst) {
			return;
		}

		let req = DownloadRequest { url, folder, filename };

		let tx = self.file_downloader.clone();
		let counter = self.pending_downloads.clone();
		let tracker = self.download_tracker.clone();

		tokio::spawn(async move {
			counter.fetch_add(1, Ordering::SeqCst);

			if let Err(e) = tracker.log_start(&req) {
				error!(?req, error = %e, "Failed to log download start. Aborting submission.");
				counter.fetch_sub(1, Ordering::SeqCst);
				return;
			}

			if tx.send(req).await.is_err() {
				warn!("Asset queue closed, failed to schedule download.");
				counter.fetch_sub(1, Ordering::SeqCst);
			}
		});
	}
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	let (non_blocking_writer, _guard) = non_blocking(std::io::stdout());
	tracing::subscriber::set_global_default(
		FmtSubscriber::builder()
			.with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
			.compact()
			.with_target(false)
			.with_timer(HumanUptime::new())
			.with_writer(non_blocking_writer)
			.finish(),
	)
	.expect("setting default subscriber failed");

	if SETTINGS.discord_token.is_empty() {
		anyhow::bail!("DISCORD_TOKEN is not set.");
	}

	let mut shard = Shard::new(ShardId::ONE, SETTINGS.discord_token.clone(), Intents::all());

	let http = Arc::new(HttpClient::new(SETTINGS.discord_token.clone()));
	let cache = Arc::new(DefaultInMemoryCache::builder().resource_types(ResourceType::all()).build());
	let shutdown = Arc::new(AtomicBool::new(false));

	let (asset_tx, asset_rx) = mpsc::channel(50_000);
	let download_tracker = Arc::new(DownloadTracker::new(&shutdown.clone())?);
	let unfinished_downloads = download_tracker.get_pending_downloads().await?;
	let pending_downloads = Arc::new(AtomicUsize::new(unfinished_downloads.len()));

	if !unfinished_downloads.is_empty() {
		info!("Re-queuing {} unfinished downloads.", unfinished_downloads.len());
		for req in unfinished_downloads {
			asset_tx.send(req).await.context("Failed to re-queue download task")?;
		}
	}

	let asset_worker = tokio::spawn(asset_downloader_worker(
		asset_rx,
		pending_downloads.clone(),
		download_tracker.clone(),
		shutdown.clone(),
	));

	let state = State::new(http, cache, asset_tx, pending_downloads.clone(), download_tracker, shutdown.clone());

	let mut guild_processors: HashMap<u64, mpsc::UnboundedSender<GuildQueueEvent>> = HashMap::new();

	info!("Bot starting...");

	loop {
		let item = tokio::select! {
			e = shard.next_event(EventTypeFlags::all()) => e,
			_ = tokio::signal::ctrl_c() => break,
		};

		// Handle Shard close
		let Some(result) = item else {
			break;
		};

		// Handle Network Error
		let Ok(event) = result else {
			warn!(source = ?result.unwrap_err(), "Gateway error");
			continue;
		};

		state.cache.update(&event);

		match event {
			Event::Ready(r) => {
				info!("🏃 Connected to {} guilds.", r.guilds.len());
			}
			Event::GuildCreate(e) => {
				dispatch_guild_event(&mut guild_processors, e.id(), GuildQueueEvent::InitialCatchup, &state);
			}
			Event::GuildDelete(e) => {
				info!(guild_id = %e.id, "Left guild. Stopping processor.");
				guild_processors.remove(&e.id.get());
			}
			_ => {
				if let Some(gid) = utils::get_event_guild_id(&event) {
					dispatch_guild_event(&mut guild_processors, gid, GuildQueueEvent::GatewayEvent(Box::new(event)), &state);
				}
			}
		}
	}

	info!("Shutting down...");
	shutdown.store(true, Ordering::SeqCst);

	let pending_count = pending_downloads.load(Ordering::SeqCst);
	if pending_count > 0 {
		warn!("{} attachment download task(s) remaining.", pending_count);
	}

	shard.close(CloseFrame::NORMAL);
	guild_processors.clear();

	drop(state);
	let _ = asset_worker.await;
	info!("👋 Goodbye!");
	Ok(())
}

fn dispatch_guild_event(
	processors: &mut HashMap<u64, mpsc::UnboundedSender<GuildQueueEvent>>,
	guild_id: Id<GuildMarker>,
	mut event: GuildQueueEvent,
	state: &State,
) {
	let gid = guild_id.get();

	if let Some(tx) = processors.get(&gid) {
		match tx.send(event) {
			Ok(()) => return,
			Err(e) => {
				event = e.0;
				processors.remove(&gid);
			}
		}
	}

	let (tx, rx) = mpsc::unbounded_channel();
	let state_clone = state.clone();
	tokio::spawn(async move {
		guild_processor_task(guild_id, rx, state_clone).await;
	});

	let _ = tx.send(event);
	processors.insert(gid, tx);
}

#[instrument(skip_all, fields(guild_id = %guild_id))]
async fn guild_processor_task(guild_id: Id<GuildMarker>, mut rx: mpsc::UnboundedReceiver<GuildQueueEvent>, state: State) {
	info!("Started guild processor task.");
	let mut meta_archiver = MetadataArchiver::new(guild_id, state.shutdown.clone()).await;
	let mut chan_archivers: HashMap<Id<ChannelMarker>, Arc<ChannelArchiver>> = HashMap::new();

	while let Some(event) = rx.recv().await {
		if state.shutdown.load(Ordering::SeqCst) {
			break;
		}
		let is_catchup = matches!(event, GuildQueueEvent::InitialCatchup);

		let res = match event {
			GuildQueueEvent::InitialCatchup => run_full_guild_catchup(guild_id, state.clone(), &mut meta_archiver).await,
			GuildQueueEvent::GatewayEvent(evt) => {
				dispatch::handle_event(*evt, guild_id, &state, &mut meta_archiver, &mut chan_archivers).await
			}
		};

		if let Err(e) = res {
			if is_catchup {
				error!(error = ?e, "FATAL error during initial catchup. Terminating task.");
				break;
			}

			match ProcessorError::from(e) {
				ProcessorError::Recoverable(err) => {
					warn!(error = ?err, "Recoverable error in guild processor.");
					tokio::time::sleep(Duration::from_secs(2)).await;
				}
				ProcessorError::Fatal(err) => {
					error!(error = ?err, "FATAL error. Terminating task.");
					break;
				}
			}
		}
	}
	info!("Guild processor task terminated");
}
