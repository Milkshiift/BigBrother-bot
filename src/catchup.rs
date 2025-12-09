use crate::State;
use crate::messages::ChannelArchiver;
use crate::metadata::MetadataArchiver;
use crate::settings::SETTINGS;
use anyhow::Context;
use futures_util::{StreamExt, stream};
use std::sync::atomic::Ordering;
use tracing::{debug, info, instrument, warn};
use twilight_model::channel::Message;
use twilight_model::id::Id;
use twilight_model::id::marker::{ChannelMarker, GuildMarker};

#[instrument(skip_all)]
pub async fn run_full_guild_catchup(
	guild_id: Id<GuildMarker>,
	state: State,
	metadata_archiver: &mut MetadataArchiver,
) -> anyhow::Result<()> {
	info!("Starting full catchup for guild.");

	metadata_archiver.do_full_catchup(&state, guild_id).await?;

	if state.shutdown.load(Ordering::SeqCst) {
		return Ok(());
	}

	run_message_catchup(guild_id, state.clone()).await?;

	let pending = state.pending_downloads.load(Ordering::SeqCst);
	info!("✅ Full catchup complete for guild. (Background downloads pending: {})", pending);

	Ok(())
}

#[instrument(skip_all)]
async fn run_message_catchup(guild_id: Id<GuildMarker>, state: State) -> anyhow::Result<()> {
	info!("Starting message catchup for guild.");

	if let Some(channels) = state.cache.guild_channels(guild_id) {
		stream::iter(channels.iter())
			.map(|&channel_id| (channel_id, state.clone()))
			.for_each_concurrent(SETTINGS.catchup.channel_concurrency, |(channel_id, state)| async move {
				if state.shutdown.load(Ordering::Relaxed) {
					return;
				}
				if let Err(e) = process_channel(channel_id, state).await {
					tracing::error!(%channel_id, error = ?e, "Failed to process channel");
				}
			})
			.await;
	} else {
		warn!("No channels found in cache for guild.");
		return Ok(());
	}

	Ok(())
}

#[instrument(skip_all, fields(channel_id=channel_id.get()))]
async fn process_channel(channel_id: Id<ChannelMarker>, state: State) -> anyhow::Result<()> {
	let channel = state.cache.channel(channel_id).context("Channel missing from cache")?;

	if !crate::utils::is_archivable_channel(channel.kind) {
		return Ok(());
	}

	let guild_id = channel.guild_id.context("Channel missing guild_id")?;
	drop(channel);
	let archiver = ChannelArchiver::new(guild_id.get(), channel_id.get(), &state.shutdown.clone())?;

	let start_after = archiver.get_last_message_id().await?.map_or_else(|| Id::new(1), Id::new);

	info!(start_after = %start_after.get(), "Starting message catchup.");

	let mut message_buffer = Vec::with_capacity(SETTINGS.catchup.write_batch_size);
	let mut current_after = start_after;

	loop {
		if state.shutdown.load(Ordering::Relaxed) {
			break;
		}

		let messages = state
			.http
			.channel_messages(channel_id)
			.limit(SETTINGS.catchup.messages_per_request)
			.after(current_after)
			.await?
			.models()
			.await?;

		if messages.is_empty() {
			break;
		}

		let batch_size = messages.len();
		// API returns newest first
		current_after = messages.first().unwrap().id;

		message_buffer.extend(messages.into_iter().rev());

		if message_buffer.len() >= SETTINGS.catchup.write_batch_size {
			flush_buffer(&mut message_buffer, &archiver, &state).await?;
		}

		if batch_size < SETTINGS.catchup.messages_per_request as usize {
			break;
		}
	}

	flush_buffer(&mut message_buffer, &archiver, &state).await?;

	info!("✅ Message catchup complete for channel");
	Ok(())
}

async fn flush_buffer(buffer: &mut Vec<Message>, archiver: &ChannelArchiver, state: &State) -> anyhow::Result<()> {
	if buffer.is_empty() {
		return Ok(());
	}

	debug!("Flushing {} messages.", buffer.len());

	let batch = std::mem::take(buffer);
	archiver.push_messages_bulk(batch, state).await?;
	archiver.flush().await?;

	Ok(())
}
