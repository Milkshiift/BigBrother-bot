use crate::settings::SETTINGS;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::fs;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;
use twilight_gateway::Event;
use twilight_model::channel::ChannelType;
use twilight_model::id::Id;
use twilight_model::id::marker::GuildMarker;

pub async fn ensure_dir(path: &Path) -> std::io::Result<()> {
	fs::create_dir_all(path).await
}

pub fn create_path(file_names: &[&str]) -> PathBuf {
	let mut path = PathBuf::from(&SETTINGS.data_path);
	path.extend(file_names);
	path
}

pub fn remove_extension(path: &Path) -> PathBuf {
	match (path.parent(), path.file_stem()) {
		(Some(parent), Some(stem)) => parent.join(stem),
		(None, Some(stem)) => PathBuf::from(stem),
		_ => path.to_path_buf(),
	}
}

pub const fn is_archivable_channel(kind: ChannelType) -> bool {
	matches!(
		kind,
		ChannelType::GuildText
			| ChannelType::GuildAnnouncement
			| ChannelType::AnnouncementThread
			| ChannelType::PublicThread
			| ChannelType::PrivateThread
			| ChannelType::GuildVoice
			| ChannelType::GuildMedia
	)
}

pub fn get_event_guild_id(event: &Event) -> Option<Id<GuildMarker>> {
	match event {
		Event::GuildUpdate(e) => Some(e.id),
		Event::GuildEmojisUpdate(e) => Some(e.guild_id),
		Event::GuildStickersUpdate(e) => Some(e.guild_id),
		Event::MessageCreate(e) => e.guild_id,
		Event::MessageUpdate(e) => e.guild_id,
		Event::MessageDelete(e) => e.guild_id,
		Event::MessageDeleteBulk(e) => e.guild_id,
		Event::ReactionAdd(e) => e.guild_id,
		Event::ReactionRemove(e) => e.guild_id,
		Event::ReactionRemoveAll(e) => e.guild_id,
		Event::ReactionRemoveEmoji(e) => Some(e.guild_id),
		Event::MemberAdd(e) => Some(e.guild_id),
		Event::MemberUpdate(e) => Some(e.guild_id),
		Event::MemberRemove(e) => Some(e.guild_id),
		Event::RoleCreate(e) => Some(e.guild_id),
		Event::RoleUpdate(e) => Some(e.guild_id),
		Event::RoleDelete(e) => Some(e.guild_id),
		Event::ChannelCreate(e) => e.guild_id,
		Event::ChannelUpdate(e) => e.guild_id,
		Event::ChannelDelete(e) => e.guild_id,
		Event::ThreadCreate(e) => e.guild_id,
		Event::ThreadUpdate(e) => e.guild_id,
		Event::ThreadDelete(e) => Some(e.guild_id),
		_ => None,
	}
}

pub fn get_current_time_millis() -> Result<u64, std::time::SystemTimeError> {
	#[allow(clippy::cast_possible_truncation)]
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map(|duration| duration.as_millis() as u64)
}

macro_rules! int_to_str {
	($num:expr, $type:ty) => {{ itoa::Buffer::new().format($num).to_string() }};
}
pub(crate) use int_to_str;

macro_rules! concat_str {
    ($capacity:expr, $($part:expr),* $(,)?) => {{
        let mut result = String::with_capacity($capacity);
        $(
            result.push_str($part);
        )*
        result
    }};
}
pub(crate) use concat_str;

pub struct HumanUptime {
	start_time: Instant,
}

impl HumanUptime {
	pub(crate) fn new() -> Self {
		Self {
			start_time: Instant::now(),
		}
	}
}

impl FormatTime for HumanUptime {
	fn format_time(&self, w: &mut Writer<'_>) -> fmt::Result {
		let elapsed = self.start_time.elapsed();

		if elapsed.as_nanos() == 0 {
			return write!(w, "0ms ");
		}

		let mut total_seconds = elapsed.as_secs();
		let millis = elapsed.subsec_millis();

		let days = total_seconds / (60 * 60 * 24);
		total_seconds %= 60 * 60 * 24;

		let hours = total_seconds / (60 * 60);
		total_seconds %= 60 * 60;

		let minutes = total_seconds / 60;
		let seconds = total_seconds % 60;

		let mut has_printed_larger_unit = days > 0;

		if has_printed_larger_unit || hours > 0 {
			write!(w, "{hours}h ")?;
			has_printed_larger_unit = true;
		}

		if has_printed_larger_unit || minutes > 0 {
			write!(w, "{minutes}m ")?;
			has_printed_larger_unit = true;
		}

		if has_printed_larger_unit || seconds > 0 {
			write!(w, "{seconds}s ")?;
		}

		if has_printed_larger_unit || seconds > 0 || millis > 0 || elapsed.as_secs() == 0 {
			write!(w, "{millis}ms ")?;
		}

		Ok(())
	}
}
