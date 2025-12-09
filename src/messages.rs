use crate::State;
use crate::storage::LogStore;
use crate::utils::{concat_str, create_path, int_to_str, remove_extension};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tracing::instrument;
use twilight_model::channel::message::{Embed, EmojiReactionType};
use twilight_model::channel::{Attachment, Message};

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub enum ReactionData {
	#[serde(rename = "c")]
	Custom(u64),
	#[serde(rename = "u")]
	Unicode(String),
}

impl From<&EmojiReactionType> for ReactionData {
	fn from(emoji: &EmojiReactionType) -> Self {
		match emoji {
			EmojiReactionType::Custom { id, .. } => Self::Custom(id.get()),
			EmojiReactionType::Unicode { name } => Self::Unicode(name.clone()),
		}
	}
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(tag = "t")]
pub enum MessageEvent {
	#[serde(rename = "c")]
	Create {
		#[serde(flatten)]
		message: StoredMessage,
	},
	#[serde(rename = "u")]
	Update {
		#[serde(flatten)]
		message: StoredMessage,
	},
	#[serde(rename = "d")]
	Delete {
		#[serde(rename = "i")]
		id: u64,
	},
	#[serde(rename = "bd")]
	BulkDelete {
		#[serde(rename = "is")]
		ids: Vec<u64>,
	},
	#[serde(rename = "ra")]
	ReactionAdd {
		#[serde(rename = "i")]
		message_id: u64,
		#[serde(rename = "u")]
		user_id: u64,
		#[serde(rename = "e")]
		emoji: ReactionData,
	},
	#[serde(rename = "rr")]
	ReactionRemove {
		#[serde(rename = "i")]
		message_id: u64,
		#[serde(rename = "u")]
		user_id: u64,
		#[serde(rename = "e")]
		emoji: ReactionData,
	},
	#[serde(rename = "rra")]
	ReactionRemoveAll {
		#[serde(rename = "i")]
		message_id: u64,
	},
	#[serde(rename = "rre")]
	ReactionRemoveEmoji {
		#[serde(rename = "i")]
		message_id: u64,
		#[serde(rename = "e")]
		emoji: ReactionData,
	},
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct StoredMessage {
	#[serde(rename = "i")]
	pub id: u64,
	#[serde(skip_serializing_if = "String::is_empty", default, rename = "ct")]
	pub content: String,
	#[serde(rename = "ca")]
	pub created_at: u64,
	#[serde(skip_serializing_if = "Option::is_none", rename = "ea")]
	pub edited_at: Option<u64>,
	#[serde(rename = "a")]
	pub author_id: u64,
	#[serde(skip_serializing_if = "Vec::is_empty", default, rename = "e")]
	pub embeds: Vec<Embed>,
	#[serde(skip_serializing_if = "Vec::is_empty", default, rename = "at")]
	pub attachments: Vec<u64>,
	#[serde(skip_serializing_if = "Vec::is_empty", default, rename = "s")]
	pub stickers: Vec<u64>,
	#[serde(skip_serializing_if = "Vec::is_empty", default, rename = "r")]
	pub reactions: Vec<(ReactionData, u64)>,
	#[serde(skip_serializing_if = "Option::is_none", rename = "ri")]
	pub reference_id: Option<u64>,
}

impl From<Message> for StoredMessage {
	fn from(mut msg: Message) -> Self {
		let id = msg.id.get();
		let created_at = (msg.timestamp.as_micros() / 1000).cast_unsigned();
		let edited_at = msg.edited_timestamp.map(|t| (t.as_micros() / 1000).cast_unsigned());
		let author_id = msg.author.id.get();
		let reference_id = msg.reference.as_ref().and_then(|r| r.message_id.map(twilight_model::id::Id::get));
		let content = std::mem::take(&mut msg.content);
		let embeds = std::mem::take(&mut msg.embeds);
		let attachments = msg.attachments.into_iter().map(|a| a.id.get()).collect();
		let stickers = msg.sticker_items.into_iter().map(|s| s.id.get()).collect();
		let reactions = std::mem::take(&mut msg.reactions)
			.into_iter()
			.map(|r| {
				let reaction_data = match &r.emoji {
					EmojiReactionType::Custom { id, .. } => ReactionData::Custom(id.get()),
					EmojiReactionType::Unicode { name } => ReactionData::Unicode(name.clone()),
				};
				(reaction_data, r.count)
			})
			.collect();

		Self {
			id,
			content,
			created_at,
			edited_at,
			author_id,
			embeds,
			attachments,
			stickers,
			reactions,
			reference_id,
		}
	}
}

/// Manages the archiving logic and state for a single channel.
pub struct ChannelArchiver {
	log_store: LogStore,
	channel_id: u64,
}

impl ChannelArchiver {
	pub fn new(guild_id: u64, channel_id: u64, shutdown: &Arc<AtomicBool>) -> Result<Self> {
		let guild_id_str = int_to_str!(guild_id, u64);
		let channel_id_str = int_to_str!(channel_id, u64);

		let path = create_path(&[&guild_id_str, "messages", &concat_str!(27, &channel_id_str, ".ndjson")]);
		let log_store = LogStore::new(path, shutdown)?;

		Ok(Self { log_store, channel_id })
	}

	#[instrument(skip(self, msg, state), fields(channel_id = %self.channel_id))]
	pub async fn push_message(&self, msg: Message, state: &State) -> Result<()> {
		let attachments = msg.attachments.clone();
		let event = MessageEvent::Create {
			message: StoredMessage::from(msg),
		};
		self.log_store.append(&event)?;

		if !attachments.is_empty() {
			let folder_path = remove_extension(self.log_store.path());
			Self::queue_attachments(state, &attachments, &folder_path);
		}
		Ok(())
	}

	#[instrument(skip(self, messages, state), fields(channel_id = %self.channel_id, count = messages.len()))]
	pub async fn push_messages_bulk(&self, messages: Vec<Message>, state: &State) -> Result<()> {
		if messages.is_empty() {
			return Ok(());
		}

		let all_attachments: Vec<Attachment> = messages.iter().flat_map(|m| m.attachments.clone()).collect();

		let events: Vec<MessageEvent> = messages
			.into_iter()
			.map(|msg| MessageEvent::Create {
				message: StoredMessage::from(msg),
			})
			.collect();
		self.log_store.append_bulk(events)?;

		if !all_attachments.is_empty() {
			let folder_path = remove_extension(self.log_store.path());
			Self::queue_attachments(state, &all_attachments, &folder_path);
		}
		Ok(())
	}

	#[instrument(skip(self, msg), fields(channel_id = %self.channel_id, message_id = %msg.id.get()))]
	pub async fn update_message(&self, msg: Message) -> Result<()> {
		let event = MessageEvent::Update {
			message: StoredMessage::from(msg),
		};
		self.log_store.append(&event)
	}

	#[instrument(skip(self), fields(channel_id = %self.channel_id, message_id))]
	pub async fn delete_message(&self, message_id: u64) -> Result<()> {
		let event = MessageEvent::Delete { id: message_id };
		self.log_store.append(&event)
	}

	#[instrument(skip(self), fields(channel_id = %self.channel_id, count = ids_to_delete.len()))]
	pub async fn mass_delete_messages(&self, ids_to_delete: &[u64]) -> Result<usize> {
		if ids_to_delete.is_empty() {
			return Ok(0);
		}
		let event = MessageEvent::BulkDelete {
			ids: ids_to_delete.to_vec(),
		};
		self.log_store.append(&event)?;
		Ok(ids_to_delete.len())
	}

	#[instrument(skip(self, emoji), fields(channel_id = %self.channel_id, message_id, user_id))]
	pub async fn add_reaction(&self, message_id: u64, user_id: u64, emoji: &EmojiReactionType) -> Result<()> {
		let event = MessageEvent::ReactionAdd {
			message_id,
			user_id,
			emoji: ReactionData::from(emoji),
		};
		self.log_store.append(&event)
	}

	#[instrument(skip(self, emoji), fields(channel_id = %self.channel_id, message_id, user_id))]
	pub async fn remove_reaction(&self, message_id: u64, user_id: u64, emoji: &EmojiReactionType) -> Result<()> {
		let event = MessageEvent::ReactionRemove {
			message_id,
			user_id,
			emoji: ReactionData::from(emoji),
		};
		self.log_store.append(&event)
	}

	#[instrument(skip(self), fields(channel_id = %self.channel_id, message_id))]
	pub async fn remove_all_reactions(&self, message_id: u64) -> Result<()> {
		let event = MessageEvent::ReactionRemoveAll { message_id };
		self.log_store.append(&event)
	}

	#[instrument(skip(self, emoji), fields(channel_id = %self.channel_id, message_id))]
	pub async fn remove_emoji_reactions(&self, message_id: u64, emoji: &EmojiReactionType) -> Result<()> {
		let event = MessageEvent::ReactionRemoveEmoji {
			message_id,
			emoji: ReactionData::from(emoji),
		};
		self.log_store.append(&event)
	}

	#[instrument(skip(self), fields(channel_id = %self.channel_id))]
	pub async fn get_last_message_id(&self) -> Result<Option<u64>> {
		// Minimal struct for efficient deserialization
		#[derive(serde::Deserialize)]
		struct ScanFrame {
			#[serde(rename = "t")]
			tag: String,
			#[serde(rename = "i")]
			id: u64,
		}

		let last_id = self
			.log_store
			.scan_last(|frame: ScanFrame| if frame.tag == "c" { Some(frame.id) } else { None })
			.await?;

		Ok(last_id)
	}

	fn queue_attachments(state: &State, attachments: &[Attachment], folder: &Path) {
		for att in attachments {
			let filename = format!("{}_{}", int_to_str!(att.id.get(), u64), att.filename);
			state.submit_download(att.url.clone(), folder.to_path_buf(), filename);
		}
	}

	pub async fn flush(&self) -> Result<()> {
		self.log_store.flush().await
	}
}
