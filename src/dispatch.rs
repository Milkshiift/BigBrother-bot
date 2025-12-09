use crate::State;
use crate::messages::ChannelArchiver;
use crate::metadata::{GuildUpdate, MetadataArchiver};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::instrument;
use twilight_gateway::Event;
use twilight_model::guild::{Member, MemberFlags};
use twilight_model::id::Id;
use twilight_model::id::marker::{ChannelMarker, GuildMarker};

fn get_archiver(
	guild_id: Id<GuildMarker>,
	channel_id: Id<ChannelMarker>,
	state: &State,
	cache: &mut HashMap<Id<ChannelMarker>, Arc<ChannelArchiver>>,
) -> anyhow::Result<Arc<ChannelArchiver>> {
	if let Some(archiver) = cache.get(&channel_id) {
		return Ok(archiver.clone());
	}

	let archiver = Arc::new(ChannelArchiver::new(guild_id.get(), channel_id.get(), &state.shutdown.clone())?);
	cache.insert(channel_id, archiver.clone());
	Ok(archiver)
}

#[instrument(skip_all, fields(event = ?event.kind()))]
pub async fn handle_event(
	event: Event,
	guild_id: Id<GuildMarker>,
	state: &State,
	meta: &mut MetadataArchiver,
	chan_archivers: &mut HashMap<Id<ChannelMarker>, Arc<ChannelArchiver>>,
) -> anyhow::Result<()> {
	macro_rules! forward {
        ($channel_id:expr, $method:ident $(, $args:expr)*) => {{
            get_archiver(guild_id, $channel_id, state, chan_archivers)?
                .$method($($args),*).await
        }};
    }

	match event {
		Event::MessageCreate(m) => forward!(m.channel_id, push_message, m.0, state)?,
		Event::MessageUpdate(m) => forward!(m.channel_id, update_message, m.0)?,
		Event::MessageDelete(m) => forward!(m.channel_id, delete_message, m.id.get())?,
		Event::MessageDeleteBulk(m) => {
			let ids: Vec<u64> = m.ids.iter().map(|id| id.get()).collect();
			forward!(m.channel_id, mass_delete_messages, &ids).map(|_| ())?;
		}

		Event::ReactionAdd(r) => forward!(r.channel_id, add_reaction, r.message_id.get(), r.user_id.get(), &r.emoji)?,
		Event::ReactionRemove(r) => forward!(r.channel_id, remove_reaction, r.message_id.get(), r.user_id.get(), &r.emoji)?,
		Event::ReactionRemoveAll(r) => forward!(r.channel_id, remove_all_reactions, r.message_id.get())?,
		Event::ReactionRemoveEmoji(r) => forward!(r.channel_id, remove_emoji_reactions, r.message_id.get(), &r.emoji)?,

		Event::GuildUpdate(e) => meta.process_guild_update(state, GuildUpdate::Partial(&e.0))?,
		Event::GuildEmojisUpdate(e) => {
			let g = state.http.guild(e.guild_id).await?.model().await?;
			meta.process_guild_update(state, GuildUpdate::Full(&g))?;
		}
		Event::GuildStickersUpdate(e) => {
			let g = state.http.guild(e.guild_id).await?.model().await?;
			meta.process_guild_update(state, GuildUpdate::Full(&g))?;
		}

		Event::MemberAdd(e) => meta.process_member_update(state, &e.member)?,
		Event::MemberRemove(e) => meta.process_member_remove(e.user.id.get())?,
		Event::MemberUpdate(e) => {
			let member = Member {
				user: e.user,
				nick: e.nick,
				avatar: e.avatar,
				roles: e.roles,
				joined_at: e.joined_at,
				premium_since: e.premium_since,
				deaf: e.deaf.unwrap_or(false),
				mute: e.mute.unwrap_or(false),
				pending: e.pending,
				communication_disabled_until: e.communication_disabled_until,
				flags: MemberFlags::empty(),
				avatar_decoration_data: None,
				banner: None,
			};
			meta.process_member_update(state, &member)?;
		}

		Event::RoleCreate(e) => meta.process_role_update(&e.role)?,
		Event::RoleUpdate(e) => meta.process_role_update(&e.role)?,
		Event::RoleDelete(e) => meta.process_role_delete(e.role_id.get())?,

		Event::ChannelCreate(e) => meta.process_channel_update(&e.0)?,
		Event::ChannelUpdate(e) => meta.process_channel_update(&e.0)?,
		Event::ChannelDelete(e) => meta.process_channel_delete(e.id.get())?,

		Event::ThreadCreate(e) => meta.process_channel_update(&e.0)?,
		Event::ThreadUpdate(e) => meta.process_channel_update(&e.0)?,
		Event::ThreadDelete(e) => meta.process_channel_delete(e.id.get())?,

		_ => {}
	}
	Ok(())
}
