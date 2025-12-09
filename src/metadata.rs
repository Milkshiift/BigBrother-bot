use crate::State;
use crate::network::append_cdn;
use crate::settings::SETTINGS;
use crate::storage::LogStore;
use crate::utils::{concat_str, create_path, get_current_time_millis, int_to_str};
use anyhow::Result;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{info, instrument};
use twilight_model::channel::Channel as TwilightChannel;
use twilight_model::channel::message::sticker::{Sticker as TwilightSticker, StickerFormatType};
use twilight_model::guild::{Emoji as TwilightEmoji, Guild as TwilightGuild, Member as TwilightMember, PartialGuild, Role as TwilightRole};
use twilight_model::id::Id;
use twilight_model::id::marker::{GuildMarker, UserMarker};
use twilight_model::util::ImageHash;

/// Event that can be replayed from a log.
pub trait Replayable {
	fn id(&self) -> u64;
	fn is_delete(&self) -> bool;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct GuildEvent {
	#[serde(rename = "n")]
	pub name: String,
	#[serde(rename = "ic", skip_serializing_if = "Option::is_none")]
	pub icon: Option<String>,
	#[serde(rename = "bn", skip_serializing_if = "Option::is_none")]
	pub banner: Option<String>,
	#[serde(rename = "d", skip_serializing_if = "Option::is_none")]
	pub description: Option<String>,
	#[serde(rename = "s", skip_serializing_if = "Option::is_none")]
	pub splash: Option<String>,
}

impl Replayable for GuildEvent {
	fn id(&self) -> u64 {
		0
	}
	fn is_delete(&self) -> bool {
		false
	}
}

impl From<&PartialGuild> for GuildEvent {
	fn from(g: &PartialGuild) -> Self {
		Self {
			name: g.name.clone(),
			icon: g.icon.map(|h| h.to_string()),
			banner: g.banner.map(|h| h.to_string()),
			description: g.description.clone(),
			splash: g.splash.map(|h| h.to_string()),
		}
	}
}

impl From<&TwilightGuild> for GuildEvent {
	fn from(g: &TwilightGuild) -> Self {
		Self {
			name: g.name.clone(),
			icon: g.icon.map(|h| h.to_string()),
			banner: g.banner.map(|h| h.to_string()),
			description: g.description.clone(),
			splash: g.splash.map(|h| h.to_string()),
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct MemberEvent {
	#[serde(rename = "i")]
	pub user_id: u64,
	#[serde(rename = "u")]
	pub username: String,
	#[serde(rename = "gn", skip_serializing_if = "Option::is_none")]
	pub global_name: Option<String>,
	#[serde(rename = "a", skip_serializing_if = "Option::is_none")]
	pub avatar: Option<String>,
	#[serde(rename = "j", skip_serializing_if = "Option::is_none")]
	pub joined_at: Option<u64>,
	#[serde(rename = "l", skip_serializing_if = "Option::is_none")]
	pub left_at: Option<u64>,
	#[serde(rename = "r", skip_serializing_if = "Vec::is_empty", default)]
	pub roles: Vec<u64>,
	#[serde(rename = "nk", skip_serializing_if = "Option::is_none")]
	pub nickname: Option<String>,
	#[serde(rename = "b", skip_serializing_if = "<&bool as std::ops::Not>::not", default)]
	pub bot: bool,
}

impl Replayable for MemberEvent {
	fn id(&self) -> u64 {
		self.user_id
	}
	fn is_delete(&self) -> bool {
		self.left_at.is_some()
	}
}

impl MemberEvent {
	pub fn from_add_or_update(m: &TwilightMember) -> Self {
		Self {
			user_id: m.user.id.get(),
			username: m.user.name.clone(),
			global_name: m.user.global_name.clone(),
			avatar: m.avatar.map(|h| h.to_string()).or_else(|| m.user.avatar.map(|h| h.to_string())),
			joined_at: m.joined_at.map(|t| t.as_micros().cast_unsigned() / 1000),
			left_at: None,
			roles: m.roles.iter().map(|r| r.get()).collect(),
			nickname: m.nick.clone(),
			bot: m.user.bot,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct RoleEvent {
	#[serde(rename = "i")]
	pub role_id: u64,
	#[serde(rename = "n")]
	pub name: String,
	#[serde(rename = "c")]
	pub color: u32,
	#[serde(rename = "p")]
	pub position: i64,
	#[serde(rename = "ps")]
	pub permissions: String,
	#[serde(rename = "h", skip_serializing_if = "<&bool as std::ops::Not>::not", default)]
	pub hoist: bool,
	#[serde(rename = "m", skip_serializing_if = "<&bool as std::ops::Not>::not", default)]
	pub mentionable: bool,
	#[serde(rename = "d", skip_serializing_if = "<&bool as std::ops::Not>::not", default)]
	pub deleted: bool,
}

impl Replayable for RoleEvent {
	fn id(&self) -> u64 {
		self.role_id
	}
	fn is_delete(&self) -> bool {
		self.deleted
	}
}

impl RoleEvent {
	pub fn from_role(r: TwilightRole) -> Self {
		Self {
			role_id: r.id.get(),
			name: r.name,
			color: r.colors.primary_color,
			position: r.position,
			permissions: r.permissions.bits().to_string(),
			hoist: r.hoist,
			mentionable: r.mentionable,
			deleted: false,
		}
	}

	pub fn from_role_ref(r: &TwilightRole) -> Self {
		Self {
			role_id: r.id.get(),
			name: r.name.clone(),
			color: r.colors.primary_color,
			position: r.position,
			permissions: r.permissions.bits().to_string(),
			hoist: r.hoist,
			mentionable: r.mentionable,
			deleted: false,
		}
	}

	pub fn from_delete(id: u64) -> Self {
		Self {
			role_id: id,
			name: String::new(),
			color: 0,
			position: 0,
			permissions: "0".into(),
			hoist: false,
			mentionable: false,
			deleted: true,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ChannelEvent {
	#[serde(rename = "i")]
	pub channel_id: u64,
	#[serde(rename = "n")]
	pub name: String,
	#[serde(rename = "t", skip_serializing_if = "Option::is_none")]
	pub topic: Option<String>,
	#[serde(rename = "ty")]
	pub channel_type: u8,
	#[serde(rename = "p")]
	pub position: i32,
	#[serde(rename = "pi", skip_serializing_if = "Option::is_none")]
	pub parent_id: Option<u64>,
	#[serde(rename = "ns", skip_serializing_if = "<&bool as std::ops::Not>::not", default)]
	pub nsfw: bool,
	#[serde(rename = "d", skip_serializing_if = "<&bool as std::ops::Not>::not", default)]
	pub deleted: bool,
}

impl Replayable for ChannelEvent {
	fn id(&self) -> u64 {
		self.channel_id
	}
	fn is_delete(&self) -> bool {
		self.deleted
	}
}

impl ChannelEvent {
	pub fn from_channel(c: TwilightChannel) -> Self {
		Self {
			channel_id: c.id.get(),
			name: c.name.unwrap_or_default(),
			topic: c.topic,
			channel_type: c.kind.into(),
			position: c.position.unwrap_or_default(),
			parent_id: c.parent_id.map(Id::get),
			nsfw: c.nsfw.unwrap_or(false),
			deleted: false,
		}
	}

	pub fn from_channel_ref(c: &TwilightChannel) -> Self {
		Self {
			channel_id: c.id.get(),
			name: c.name.clone().unwrap_or_default(),
			topic: c.topic.clone(),
			channel_type: c.kind.into(),
			position: c.position.unwrap_or_default(),
			parent_id: c.parent_id.map(Id::get),
			nsfw: c.nsfw.unwrap_or(false),
			deleted: false,
		}
	}

	pub fn from_delete(id: u64) -> Self {
		Self {
			channel_id: id,
			name: "DELETED".into(),
			topic: None,
			channel_type: 0,
			position: 0,
			parent_id: None,
			nsfw: false,
			deleted: true,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmojiEvent {
	#[serde(rename = "i")]
	pub id: u64,
	#[serde(rename = "n")]
	pub name: String,
	#[serde(rename = "a")]
	pub animated: bool,
	#[serde(rename = "d", skip_serializing_if = "<&bool as std::ops::Not>::not", default)]
	pub deleted: bool,
}

impl Replayable for EmojiEvent {
	fn id(&self) -> u64 {
		self.id
	}
	fn is_delete(&self) -> bool {
		self.deleted
	}
}

impl EmojiEvent {
	pub fn from_api(e: &TwilightEmoji) -> Self {
		Self {
			id: e.id.get(),
			name: e.name.clone(),
			animated: e.animated,
			deleted: false,
		}
	}
	pub const fn from_delete(id: u64) -> Self {
		Self {
			id,
			name: String::new(),
			animated: false,
			deleted: true,
		}
	}
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StickerEvent {
	#[serde(rename = "i")]
	pub id: u64,
	#[serde(rename = "n")]
	pub name: String,
	#[serde(rename = "f")]
	pub format_type: StickerFormatType,
	#[serde(rename = "d", skip_serializing_if = "<&bool as std::ops::Not>::not", default)]
	pub deleted: bool,
}

impl Replayable for StickerEvent {
	fn id(&self) -> u64 {
		self.id
	}
	fn is_delete(&self) -> bool {
		self.deleted
	}
}

impl StickerEvent {
	pub fn from_api(s: &TwilightSticker) -> Self {
		Self {
			id: s.id.get(),
			name: s.name.clone(),
			format_type: s.format_type,
			deleted: false,
		}
	}
	pub const fn from_delete(id: u64) -> Self {
		Self {
			id,
			name: String::new(),
			format_type: StickerFormatType::Png,
			deleted: true,
		}
	}
}

#[derive(Debug)]
struct EntityManager<T> {
	state: HashMap<u64, T>,
	log_store: LogStore,
}

impl<T> EntityManager<T>
where
	T: Serialize + DeserializeOwned + Clone + PartialEq + Send + Sync + Replayable + 'static,
{
	async fn new(guild_id: Id<GuildMarker>, entity_type: &'static str, shutdown: Arc<AtomicBool>) -> Self {
		let path = create_path(&[&guild_id.to_string(), "metadata", &concat_str!(16, &entity_type, ".ndjson")]);
		let log_store = LogStore::new(path, &shutdown).expect("Failed to create log store");

		let mut state = HashMap::new();
		if let Ok(events) = log_store.read_all::<T>().await {
			for event in events {
				let data = event.payload;
				if data.is_delete() {
					state.remove(&data.id());
				} else {
					state.insert(data.id(), data);
				}
			}
		}

		Self { state, log_store }
	}

	fn handle_update(&mut self, id: u64, data: T) -> Result<bool> {
		if self.state.get(&id) == Some(&data) {
			return Ok(false);
		}
		self.log_store.append(&data)?;
		self.state.insert(id, data);
		Ok(true)
	}

	fn handle_delete(&mut self, id: u64, delete_event_generator: impl FnOnce() -> T) -> Result<()> {
		if self.state.contains_key(&id) {
			self.log_store.append(&delete_event_generator())?;
			self.state.remove(&id);
		}
		Ok(())
	}

	/// Reconciles the local state with a list of items from the API.
	/// Updates existing/new items and deletes items not present in the API list.
	fn reconcile<U, FMap, FDel>(&mut self, api_items: Vec<U>, map_fn: FMap, delete_fn: FDel) -> Result<()>
	where
		FMap: Fn(U) -> T,
		FDel: Fn(u64) -> T,
	{
		let mut seen_ids = HashSet::new();
		for item in api_items {
			let event = map_fn(item);
			let id = event.id();
			seen_ids.insert(id);
			self.handle_update(id, event)?;
		}

		let cached_ids: Vec<u64> = self.state.keys().copied().collect();
		for id in cached_ids {
			if !seen_ids.contains(&id) {
				self.handle_delete(id, || delete_fn(id))?;
			}
		}
		Ok(())
	}
}

#[derive(Debug, Clone, Copy)]
pub enum GuildUpdate<'a> {
	Partial(&'a PartialGuild),
	Full(&'a TwilightGuild),
}

#[derive(Debug)]
pub struct MetadataArchiver {
	guild_id_str: String,
	members: EntityManager<MemberEvent>,
	roles: EntityManager<RoleEvent>,
	channels: EntityManager<ChannelEvent>,
	guild_info: EntityManager<GuildEvent>,
	emojis: EntityManager<EmojiEvent>,
	stickers: EntityManager<StickerEvent>,
}

impl MetadataArchiver {
	pub async fn new(guild_id: Id<GuildMarker>, shutdown: Arc<AtomicBool>) -> Self {
		let (members, roles, channels, guild_info, emojis, stickers) = tokio::join!(
			EntityManager::new(guild_id, "members", shutdown.clone()),
			EntityManager::new(guild_id, "roles", shutdown.clone()),
			EntityManager::new(guild_id, "channels", shutdown.clone()),
			EntityManager::new(guild_id, "guild", shutdown.clone()),
			EntityManager::new(guild_id, "emojis", shutdown.clone()),
			EntityManager::new(guild_id, "stickers", shutdown.clone()),
		);

		Self {
			guild_id_str: int_to_str!(guild_id.get(), u64),
			members,
			roles,
			channels,
			guild_info,
			emojis,
			stickers,
		}
	}

	fn asset_path(&self, folder: &str) -> PathBuf {
		create_path(&[&self.guild_id_str, "assets", folder])
	}

	pub fn process_guild_update(&mut self, state: &State, update: GuildUpdate<'_>) -> Result<()> {
		let event: GuildEvent = match update {
			GuildUpdate::Partial(p) => p.into(),
			GuildUpdate::Full(f) => f.into(),
		};

		if self.guild_info.handle_update(0, event)?
			&& let GuildUpdate::Full(guild) = update
		{
			self.queue_guild_assets(state, guild);
		}

		if let GuildUpdate::Full(guild) = update {
			self.sync_emojis(state, &guild.emojis)?;
			self.sync_stickers(state, &guild.stickers)?;
		}
		Ok(())
	}

	pub fn process_channel_update(&mut self, channel: &TwilightChannel) -> Result<()> {
		self.channels
			.handle_update(channel.id.get(), ChannelEvent::from_channel_ref(channel))?;
		Ok(())
	}

	pub fn process_channel_delete(&mut self, channel_id: u64) -> Result<()> {
		self.channels.handle_delete(channel_id, || ChannelEvent::from_delete(channel_id))?;
		Ok(())
	}

	pub fn process_role_update(&mut self, role: &TwilightRole) -> Result<()> {
		self.roles.handle_update(role.id.get(), RoleEvent::from_role_ref(role))?;
		Ok(())
	}

	pub fn process_role_delete(&mut self, role_id: u64) -> Result<()> {
		self.roles.handle_delete(role_id, || RoleEvent::from_delete(role_id))?;
		Ok(())
	}

	pub fn process_member_update(&mut self, state: &State, member: &TwilightMember) -> Result<()> {
		let event = MemberEvent::from_add_or_update(member);
		if self.members.handle_update(member.user.id.get(), event)? {
			self.queue_avatar(state, member.user.id, member.avatar.or(member.user.avatar));
		}
		Ok(())
	}

	pub fn process_member_remove(&mut self, user_id: u64) -> Result<()> {
		let ts = get_current_time_millis()?;
		self.members.handle_delete(user_id, || MemberEvent {
			user_id,
			username: "UNKNOWN".into(),
			global_name: None,
			avatar: None,
			joined_at: None,
			left_at: Some(ts),
			roles: vec![],
			nickname: None,
			bot: false,
		})?;
		Ok(())
	}

	#[instrument(skip_all)]
	pub async fn do_full_catchup(&mut self, state: &State, guild_id: Id<GuildMarker>) -> Result<()> {
		info!("Starting full metadata catchup");

		let (channels, roles, guild) = tokio::try_join!(
			state.http.guild_channels(guild_id).into_future(),
			state.http.roles(guild_id).into_future(),
			state.http.guild(guild_id).into_future()
		)?;

		self.channels
			.reconcile(channels.models().await?, ChannelEvent::from_channel, ChannelEvent::from_delete)?;
		self.roles
			.reconcile(roles.models().await?, RoleEvent::from_role, RoleEvent::from_delete)?;

		let guild_model = guild.model().await?;
		self.process_guild_update(state, GuildUpdate::Full(&guild_model))?;
		if state.shutdown.load(Ordering::SeqCst) {
			return Ok(());
		}
		self.sync_members_iterative(state, guild_id).await?;

		info!("Metadata catchup complete.");
		Ok(())
	}

	async fn sync_members_iterative(&mut self, state: &State, guild_id: Id<GuildMarker>) -> Result<()> {
		let mut after = Id::new(1);
		let mut seen_members = HashSet::new();
		let mut total = 0;

		loop {
			if state.shutdown.load(Ordering::SeqCst) {
				break;
			}
			let members = state
				.http
				.guild_members(guild_id)
				.limit(SETTINGS.metadata.member_fetch_limit)
				.after(after)
				.await?
				.models()
				.await?;
			if members.is_empty() {
				break;
			}

			after = members.last().unwrap().user.id;
			total += members.len();

			for member in members {
				if state.shutdown.load(Ordering::SeqCst) {
					break;
				}
				seen_members.insert(member.user.id.get());
				self.process_member_update(state, &member)?;
			}
		}

		let cached: Vec<u64> = self.members.state.keys().copied().collect();
		for id in cached {
			if !seen_members.contains(&id) {
				self.process_member_remove(id)?;
			}
		}
		info!("Synced {} members.", total);
		Ok(())
	}

	fn sync_emojis(&mut self, state: &State, emojis: &[TwilightEmoji]) -> Result<()> {
		let mut seen = HashSet::new();
		for emoji in emojis {
			seen.insert(emoji.id.get());
			let evt = EmojiEvent::from_api(emoji);
			if self.emojis.handle_update(emoji.id.get(), evt)? {
				let id = int_to_str!(emoji.id.get(), u64);
				let ext = if emoji.animated { ".gif" } else { ".png" };
				state.submit_download(
					append_cdn(&["emojis/", &id, ext]),
					self.asset_path("emojis"),
					concat_str!(36, &id, ext),
				);
			}
		}
		let cached: Vec<u64> = self.emojis.state.keys().copied().collect();
		for id in cached {
			if !seen.contains(&id) {
				self.emojis.handle_delete(id, || EmojiEvent::from_delete(id))?;
			}
		}
		Ok(())
	}

	fn sync_stickers(&mut self, state: &State, stickers: &[TwilightSticker]) -> Result<()> {
		let mut seen = HashSet::new();
		for sticker in stickers {
			seen.insert(sticker.id.get());
			let evt = StickerEvent::from_api(sticker);
			if self.stickers.handle_update(sticker.id.get(), evt)? {
				let id = int_to_str!(sticker.id.get(), u64);
				let ext = match sticker.format_type {
					StickerFormatType::Png | StickerFormatType::Apng => ".png",
					StickerFormatType::Lottie => ".json",
					StickerFormatType::Gif => ".gif",
					_ => ".bin",
				};
				state.submit_download(
					append_cdn(&["stickers/", &id, ext]),
					self.asset_path("stickers"),
					concat_str!(36, &id, ext),
				);
			}
		}
		let cached: Vec<u64> = self.stickers.state.keys().copied().collect();
		for id in cached {
			if !seen.contains(&id) {
				self.stickers.handle_delete(id, || StickerEvent::from_delete(id))?;
			}
		}
		Ok(())
	}

	fn queue_avatar(&self, state: &State, user_id: Id<UserMarker>, hash: Option<ImageHash>) {
		if let Some(h) = hash {
			let user_id = int_to_str!(user_id.get(), u64);
			let ext = if h.is_animated() { ".gif" } else { ".png" };
			state.submit_download(
				append_cdn(&["avatars/", &user_id, "/", &h.to_string(), ext]),
				self.asset_path("avatars"),
				concat_str!(57, &user_id, "_", &h.to_string(), ext),
			);
		}
	}

	fn queue_guild_assets(&self, state: &State, guild: &TwilightGuild) {
		let gid = &self.guild_id_str;
		if let Some(icon) = guild.icon {
			let ext = if icon.is_animated() { ".gif" } else { ".png" };
			state.submit_download(
				append_cdn(&["icons/", gid, "/", &icon.to_string(), ext]),
				self.asset_path("icons"),
				concat_str!(36, &icon.to_string(), ext),
			);
		}
		if let Some(banner) = guild.banner {
			let ext = if banner.is_animated() { ".gif" } else { ".png" };
			state.submit_download(
				append_cdn(&["banners/", gid, "/", &banner.to_string(), ext]),
				self.asset_path("banners"),
				concat_str!(36, &banner.to_string(), ext),
			);
		}
		if let Some(splash) = guild.splash {
			state.submit_download(
				append_cdn(&["splashes/", gid, "/", &splash.to_string(), ".png"]),
				self.asset_path("splashes"),
				concat_str!(36, &splash.to_string(), ".png"),
			);
		}
	}
}
