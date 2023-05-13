/*
   Copyright 2023-present CyanoJ

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.
*/

use super::{Context, Error};
use crate::{
    check_mod_role,
    entities::{prelude::*, *},
};
use image::io::Reader as ImageReader;
use image_hasher::ImageHash;
use poise::serenity_prelude as serenity;
use sea_orm::*;
use serenity::model::channel::ReactionType;
use serenity::Mentionable;
use std::{borrow::Cow, boxed::Box, io::Cursor};
use tracing::{info, instrument};

use super::{t, ContainBytes, EMOJI};

const UNKNOWN_EMOJI: isize = 10014;

#[derive(FromQueryResult)]
struct BlockImageServerData {
    mod_role: i64,
}

#[derive(FromQueryResult)]
struct ScanImageServerData {
    blocked_images: Option<Vec<u8>>,
}

struct HashData<'a> {
    hashes: Option<Vec<ImageHash>>,
    loaded: bool,
    guild: serenity::GuildId,
    data: &'a super::Data,
}

impl<'a> HashData<'a> {
    fn new(guild: serenity::GuildId, data: &'a super::Data) -> Self {
        Self {
            hashes: None,
            loaded: false,
            guild,
            data,
        }
    }

    async fn check(&mut self, text: Option<&str>) -> Option<ImageHash> {
        if let Some(text) = text {
            if let Ok(response) = t(self.data.reqwest.get(text).send().await) {
                // Add unwrap_tracing macro
                let img = t(t(
                    ImageReader::new(Cursor::new(t(response.bytes().await).ok()?))
                        .with_guessed_format(),
                )
                .ok()?
                .decode())
                .ok()?;

                let hash = self.data.hasher.hash_image(&img);
                if self.get().await.is_some_and(|x| x.contains(&hash)) {
                    return Some(hash);
                }
            }
        }
        None
    }

    async fn get(&mut self) -> Option<&Vec<ImageHash>> {
        if !self.loaded {
            self.loaded = true;

            let mut real_hashes: Vec<ImageHash> = vec![];
            if let Some(raw_hashes) = t(Servers::find_by_id(self.guild.as_u64().repack())
                .select_only()
                .column(servers::Column::Id)
                .column(servers::Column::BlockedImages)
                .into_model::<ScanImageServerData>()
                .one(&self.data.db)
                .await)
            .ok()?
            .and_then(|m| m.blocked_images)
            {
                let raw_hash_slices: &[u8] = &raw_hashes;
                for i in raw_hash_slices.chunks_exact(super::HASH_BYTES.into()) {
                    real_hashes
                        .push(t(ImageHash::from_bytes(i).map_err(|x| format!("{x:?}"))).ok()?);
                }
                self.hashes = Some(real_hashes);
            }
        }
        self.hashes.as_ref()
    }

    async fn retrieve(mut self) -> Option<Vec<ImageHash>> {
        self.get().await;
        self.hashes
    }
}

macro_rules! impl_ref {
    (impl $trait:ident for $type:ty {
        $(fn $name:ident $params:tt -> $ret:ty $body:block)*
    }) => {
        impl $trait for $type {
            $(fn $name $params -> $ret $body)*
        }

        impl $trait for &mut $type {
            $(fn $name $params -> $ret $body)*
        }

        impl $trait for & $type {
            $(fn $name $params -> $ret $body)*
        }
    }
}

#[derive(Clone, Copy)]
pub enum ResolveUrl<'a> {
    Direct(&'a str),
    Emoji(serenity::EmojiId),
    Sticker(&'a serenity::StickerItem),
    Reaction(&'a serenity::MessageReaction),
    Icon(&'a str),
    Banner(&'a str),
}

impl<'a> ResolveUrl<'a> {
    fn resolve(&self) -> Option<Cow<'a, str>> {
        match self {
            Self::Emoji(id) => Some(Cow::Owned(format!(
                "https://cdn.discordapp.com/emojis/{}",
                id.as_u64()
            ))),
            Self::Sticker(sticker) => sticker.image_url().map(Cow::Owned),
            Self::Reaction(reaction) => match reaction.reaction_type {
                ReactionType::Custom { id, .. } => Some(Self::Emoji(id).resolve()),
                _ => None,
            }
            .flatten(),
            Self::Direct(text) | Self::Icon(text) | Self::Banner(text) => Some(Cow::Borrowed(text)),
        }
    }
}

pub trait Filterable {
    fn get_urls(&self) -> Vec<ResolveUrl>;
}

impl_ref! {
impl Filterable for serenity::Message {
    fn get_urls(&self) -> Vec<ResolveUrl> {
        vec![
            EMOJI.captures_iter(&self.content).map(|x| x.get(3).and_then(|y| t(y.as_str().parse()).ok().map(serenity::EmojiId))
            ).filter_map(|x| x.map(ResolveUrl::Emoji)).collect::<Vec<ResolveUrl>>(),
            self.attachments
                .iter()
                .map(|x| ResolveUrl::Direct(x.url.as_str()))
                .collect::<Vec<ResolveUrl>>(),
            self.embeds
                .iter()
                .flat_map(|x| {
                    [
                        x.author
                            .as_ref()
                            .and_then(|y| y.icon_url.as_deref()),
                        x.image.as_ref().map(|y| y.url.as_str()),
                        x.footer
                            .as_ref()
                            .and_then(|y| y.icon_url.as_deref()),
                        x.thumbnail.as_ref().map(|y| y.url.as_str()),
                    ]
                })
                .filter_map(|x| x.map(ResolveUrl::Direct))
                .collect::<Vec<ResolveUrl>>(),
        ]
        .concat()
    }
}
}

impl_ref! {
impl Filterable for &serenity::MessageUpdateEvent {
    fn get_urls(&self) -> Vec<ResolveUrl> {
        vec![
            self.content.as_ref().map(|i|
            EMOJI.captures_iter(i).map(|x| x.get(3).and_then(|y| t(y.as_str().parse()).ok().map(serenity::EmojiId))
            ).filter_map(|x| x.map(ResolveUrl::Emoji)).collect::<Vec<ResolveUrl>>()),

            self.attachments
                .as_ref()
                .map(|i| i.iter().map(|x| ResolveUrl::Direct(x.url.as_str())).collect::<Vec<ResolveUrl>>()),
            self.embeds.as_ref().map(|i| {
                i.iter()
                    .flat_map(|x| {
                        [
                            x.author
                                .as_ref()
                                .and_then(|y| y.icon_url.as_deref()),
                            x.image.as_ref().map(|y| y.url.as_str()),
                            x.footer
                                .as_ref()
                                .and_then(|y| y.icon_url.as_deref()),
                            x.thumbnail.as_ref().map(|y| y.url.as_str()),
                        ]
                    })
                    .filter_map(|x| x.map(ResolveUrl::Direct))
                    .collect::<Vec<ResolveUrl>>()
            }),
        ]
        .into_iter()
        .flatten()
        .flatten()
        .collect()
    }
}
}

#[instrument(skip_all, err)]
pub async fn filter_message<T: Filterable>(
    filter: T,
    guild: serenity::GuildId,
    channel: serenity::ChannelId,
    id: serenity::MessageId,
    author: &serenity::User,
    reference: super::EventReference<'_>,
) -> Result<bool, super::Error> {
    let mut hash_struct = HashData::new(guild, reference.3);

    for i in filter.get_urls() {
        if let Some(x) = hash_struct
            .check(i.resolve().as_ref().map(AsRef::as_ref))
            .await
        {
            channel.delete_message(&reference.0, id).await?;
            channel
                .send_message(&reference.0, |f| {
                    f.content(format!(
                        "Deleted message from {} (reason: blocked image)",
                        author.mention()
                    ))
                })
                .await?;
            info!(
                "Deleted blocked image from '{}#{}' (hash: '{}')",
                author.name,
                author.discriminator,
                x.to_base64()
            );
            return Ok(true);
        }
    }

    Ok(false)
}

#[instrument(skip_all, err)]
pub async fn filter_stickers(
    stickers: Vec<serenity::Sticker>,
    guild: serenity::GuildId,
    reference: super::EventReference<'_>,
) -> Result<(), super::Error> {
    let mut hash_struct = HashData::new(guild, reference.3);

    for i in stickers {
        if let Some(url) = i.image_url() {
            if let Some(hash) = hash_struct.check(Some(&url)).await {
                i.delete(reference.0).await?;
                info!("Deleted sticker! (hash: '{}')", hash.to_base64());
            }
        }
    }
    Ok(())
}

#[instrument(skip_all, err)]
pub async fn filter_member(
    member: &serenity::Member,
    guild: serenity::GuildId,
    reference: super::EventReference<'_>,
) -> Result<(), super::Error> {
    let mut hash_struct = HashData::new(guild, reference.3);

    if let Some(hash) = hash_struct.check(Some(&member.face())).await {
        kick_blocked_user(reference.0, guild, member.user.id).await?;
        info!("Kicked user for image (hash: '{}')", hash.to_base64());
    }
    Ok(())
}

#[instrument(skip_all, err)]
pub async fn filter_server(
    server: &serenity::PartialGuild,
    mut guild: serenity::GuildId,
    reference: super::EventReference<'_>,
) -> Result<(), super::Error> {
    let mut hash_struct = HashData::new(guild, reference.3);

    if let Some(hash) = hash_struct.check(server.icon_url().as_deref()).await {
        guild.edit(reference.0, |f| f.icon(None)).await?;
        info!(
            "Removed blocked image from server icon (hash: '{}')",
            hash.to_base64()
        );
    }

    if let Some(hash) = hash_struct.check(server.banner_url().as_deref()).await {
        guild.edit(reference.0, |f| f.banner(None)).await?;
        info!(
            "Removed blocked image from server banner (hash: '{}')",
            hash.to_base64()
        );
    }
    Ok(())
}

#[instrument(skip_all, err)]
pub async fn filter_emojis(
    stickers: Vec<serenity::Emoji>,
    guild: serenity::GuildId,
    reference: super::EventReference<'_>,
) -> Result<(), super::Error> {
    let mut hash_struct = HashData::new(guild, reference.3);

    for i in stickers {
        if let Some(hash) = hash_struct.check(Some(&i.url())).await {
            i.delete(reference.0).await?;
            info!("Deleted emoji! (hash: '{}')", hash.to_base64());
        }
    }
    Ok(())
}

#[instrument(skip_all, err)]
pub async fn filter_reaction(
    reaction: &serenity::Reaction,
    guild: serenity::GuildId,
    reference: super::EventReference<'_>,
) -> Result<(), super::Error> {
    let mut hash_struct = HashData::new(guild, reference.3);

    if let ReactionType::Custom { id, .. } = reaction.emoji {
        if let Some(hash) = hash_struct
            .check(ResolveUrl::Emoji(id).resolve().as_ref().map(AsRef::as_ref))
            .await
        {
            reaction.delete(reference.0).await?;
            info!("Deleted reaction! (hash: '{}')", hash.to_base64());
        }
    }
    Ok(())
}

/// Block an image
#[instrument(skip_all, err)]
#[poise::command(context_menu_command = "Block Image(s) or Reaction(s)", guild_only)]
pub async fn block_msg(ctx: Context<'_>, msg: serenity::Message) -> Result<(), Error> {
    let guild = ctx
        .guild()
        .ok_or(super::FedBotError::new("message not in guild"))?
        .id;

    let server_data: BlockImageServerData = Servers::find_by_id(guild.as_u64().repack())
        .select_only()
        .column(servers::Column::Id)
        .column(servers::Column::ModRole)
        .into_model()
        .one(&ctx.data().db)
        .await?
        .ok_or(super::FedBotError::new("Failed to find query"))?;
    let (mod_role,) = (serenity::RoleId(server_data.mod_role.repack()),);

    check_mod_role!(ctx, guild, mod_role);

    crate::defer!(ctx);

    let mut urls = msg.get_urls();
    for i in &msg.sticker_items {
        urls.push(ResolveUrl::Sticker(i));
    }

    for i in &msg.reactions {
        if let ReactionType::Custom { .. } = &i.reaction_type {
            urls.push(ResolveUrl::Reaction(i));
        }
    }

    if urls.is_empty() {
        ctx.send(|f| {
            f.content("No image(s) found!")
                .ephemeral(ctx.data().is_ephemeral)
        })
        .await?;
        return Ok(());
    }

    confirm_blocks(ctx, guild, Some(msg.id), None, urls).await?;
    Ok(())
}

/// Block the server icon or banner
#[instrument(skip_all, err)]
#[poise::command(slash_command, rename = "block_icon", guild_only)]
pub async fn block_server(ctx: Context<'_>) -> Result<(), Error> {
    let guild = ctx
        .guild()
        .ok_or(super::FedBotError::new("message not in guild"))?
        .id;

    let server_data: BlockImageServerData = Servers::find_by_id(guild.as_u64().repack())
        .select_only()
        .column(servers::Column::Id)
        .column(servers::Column::ModRole)
        .into_model()
        .one(&ctx.data().db)
        .await?
        .ok_or(super::FedBotError::new("Failed to find query"))?;
    let (mod_role,) = (serenity::RoleId(server_data.mod_role.repack()),);

    check_mod_role!(ctx, guild, mod_role);

    crate::defer!(ctx);

    let mut urls = vec![];
    let partial = guild.to_partial_guild(ctx).await?;
    let (maybe_icon, maybe_banner) = (partial.icon_url(), partial.banner_url());

    if let Some(x) = maybe_icon.as_deref() {
        urls.push(ResolveUrl::Icon(x));
    }
    if let Some(x) = maybe_banner.as_deref() {
        urls.push(ResolveUrl::Banner(x));
    }

    if urls.is_empty() {
        ctx.send(|f| {
            f.content("No image(s) found!")
                .ephemeral(ctx.data().is_ephemeral)
        })
        .await?;
        return Ok(());
    }

    confirm_blocks(ctx, guild, None, None, urls).await?;
    Ok(())
}

/// Block an profile picture
#[instrument(skip_all, err)]
#[poise::command(context_menu_command = "Block Profile Picture", guild_only)]
pub async fn block_pfp(ctx: Context<'_>, user: serenity::User) -> Result<(), Error> {
    let guild = ctx
        .guild()
        .ok_or(super::FedBotError::new("message not in guild"))?
        .id;

    let server_data: BlockImageServerData = Servers::find_by_id(guild.as_u64().repack())
        .select_only()
        .column(servers::Column::Id)
        .column(servers::Column::ModRole)
        .into_model()
        .one(&ctx.data().db)
        .await?
        .ok_or(super::FedBotError::new("Failed to find query"))?;
    let (mod_role,) = (serenity::RoleId(server_data.mod_role.repack()),);

    check_mod_role!(ctx, guild, mod_role);

    crate::defer!(ctx);

    let pfp_url = user.face();

    let urls = vec![ResolveUrl::Direct(&pfp_url)];

    confirm_blocks(ctx, guild, None, Some(user.id), urls).await?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
async fn confirm_blocks(
    ctx: super::Context<'_>,
    guild: serenity::GuildId,
    msg: Option<serenity::MessageId>,
    user: Option<serenity::UserId>,
    urls: Vec<ResolveUrl<'_>>,
) -> Result<(), super::Error> {
    let mut responses = vec![];
    // let mut handles = vec![];
    for (index, i) in urls.iter().enumerate() {
        if let Some(url) = i.resolve() {
            responses.push(
                ctx.send(|f| {
                    f.components(|f| {
                        f.create_action_row(|f| {
                            f.create_button(|f| {
                                f.custom_id(format!("{index}-block"))
                                    .style(serenity::ButtonStyle::Danger)
                                    .label("Block")
                            })
                            .create_button(|f| {
                                f.custom_id(format!("{index}-keep"))
                                    .style(serenity::ButtonStyle::Success)
                                    .label("Keep")
                            })
                        })
                    })
                    .embed(|f| f.image(url))
                    .ephemeral(ctx.data().is_ephemeral)
                })
                .await?,
            );
        }
    }
    if responses.is_empty() {
        return Ok(());
    }

    // let http: serenity::Http = ctx.into();

    // for i in &responses {
    //     handles.push(tokio::spawn(get_response(
    //         i.message()
    //             .await?
    //             .await_component_interaction(ctx)
    //             .author_id(ctx.author().id)
    //             .timeout(tokio::time::Duration::from_secs(15)),
    //     )));
    // }

    let mut interactions = tokio::task::JoinSet::new();

    let http = &ctx.serenity_context().http;

    for i in &responses {
        interactions.spawn(get_response(
            http.clone(),
            i.message()
                .await?
                .await_component_interaction(ctx)
                .author_id(ctx.author().id), // .timeout(tokio::time::Duration::from_secs(15)),
        ));
    }

    let mut new_hashes: Vec<u8> = vec![];
    let old_hashes = HashData::new(guild, ctx.data()).retrieve().await;
    let mut hashes_changed = false;
    let mut msg_deleted = false;
    let mut indexes_to_delete = vec![];
    while let Some(i) = interactions.join_next().await {
        if let Some((index, to_delete)) = i? {
            if let Some(msg) = responses.get(index) {
                msg.delete(ctx).await?;
            }
            if to_delete {
                indexes_to_delete.push(index);
            }
        }
    }

    for index in indexes_to_delete {
        if let Some(resolve) = urls.get(index) {
            if let Some(url) = &resolve.resolve() {
                let hash =
                    hash_and_delete(ctx, msg, user, &mut msg_deleted, guild, url, resolve).await?;
                if !old_hashes.as_ref().is_some_and(|x| x.contains(&hash)) {
                    hashes_changed = true;
                    info!(
                        "Added new blocked image (blocker: '{}') (hash: '{}')",
                        ctx.author().tag(),
                        hash.to_base64()
                    );
                    new_hashes.extend_from_slice(hash.as_bytes());
                }
            }
        }
    }

    if let Some(msg) = msg {
        if msg_deleted {
            let author = ctx.channel_id().message(ctx, msg).await?.author.mention();
            ctx.channel_id()
                .send_message(ctx, |f| {
                    f.content(format!(
                        "Deleted message from {author} (reason: blocked image)",
                    ))
                })
                .await?;
            ctx.channel_id().delete_message(ctx, msg).await?;
        }
    }

    if !hashes_changed {
        ctx.send(|f| {
            f.content("No images blocked.")
                .ephemeral(ctx.data().is_ephemeral)
        })
        .await?;
        return Ok(());
    }

    if let Some(hashes) = old_hashes {
        for i in hashes {
            new_hashes.extend_from_slice(i.as_bytes());
        }
    }
    let mut model: servers::ActiveModel = sea_orm::ActiveModelTrait::default();
    model.id = ActiveValue::Unchanged(guild.as_u64().repack());
    model.blocked_images = ActiveValue::Set(Some(new_hashes));
    model.update(&ctx.data().db).await?;

    ctx.send(|f| {
        f.content("Added image(s) to blocklist!")
            .ephemeral(ctx.data().is_ephemeral)
    })
    .await?;

    Ok(())
}

async fn hash_and_delete(
    ctx: Context<'_>,
    msg: Option<serenity::MessageId>,
    user: Option<serenity::UserId>,
    msg_to_be_deleted: &mut bool,
    mut guild: serenity::GuildId,
    url: &str,
    resolve: &ResolveUrl<'_>,
) -> Result<ImageHash, Error> {
    let img = ImageReader::new(Cursor::new(
        ctx.data().reqwest.get(url).send().await?.bytes().await?,
    ))
    .with_guessed_format()?
    .decode()?;

    let hash = ctx.data().hasher.hash_image(&img);

    match resolve {
        ResolveUrl::Emoji(id) => match guild.emoji(ctx, *id).await {
            Ok(e) => {
                e.delete(ctx).await?;
                if let Some(user) = e.user {
                    info!(
                        "Deleted newly blocked emoji from '{}#{}' (hash: '{}')",
                        user.name,
                        user.discriminator,
                        hash.to_base64()
                    );
                } else {
                    info!("Deleted newly blocked emoji (hash: '{}')", hash.to_base64());
                }
            }
            Err(e) => {
                let mut handled: bool = false;
                if let serenity::SerenityError::Http(container) = &e {
                    if let serenity::HttpError::UnsuccessfulRequest(x) = &**container {
                        if x.error.code == UNKNOWN_EMOJI {
                            handled = true;
                            info!(
                                "Cannot delete newly blocked external emoji (hash: '{}')",
                                hash.to_base64()
                            );
                        }
                    }
                }
                if !handled {
                    return Err(e.into());
                }
            }
        },
        ResolveUrl::Direct(_) => {
            if msg.is_some() {
                *msg_to_be_deleted = true;
            }
            if let Some(user) = user {
                kick_blocked_user(ctx, guild, user).await?;
                info!("Kicked user for image (hash: '{}')", hash.to_base64());
            }
        }
        ResolveUrl::Sticker(sticker) => {
            if let Ok(x) = t(sticker.to_sticker(ctx).await) {
                t(x.delete(ctx).await).ok();
                info!("Deleted sticker (hash: '{}')", hash.to_base64());
            }
        }
        ResolveUrl::Reaction(reaction) => {
            if let Some(msg) = msg {
                ctx.channel_id()
                    .delete_reaction_emoji(ctx, msg, reaction.reaction_type.clone())
                    .await?;
                info!("Deleted reaction (hash: '{}')", hash.to_base64());
            }
        }
        ResolveUrl::Icon(_) => {
            guild.edit(ctx, |f| f.icon(None)).await?;
            info!(
                "Removed blocked image from server icon (hash: '{}')",
                hash.to_base64()
            );
        }
        ResolveUrl::Banner(_) => {
            guild.edit(ctx, |f| f.banner(None)).await?;
            info!(
                "Removed blocked image from server banner (hash: '{}')",
                hash.to_base64()
            );
        }
    };
    Ok(hash)
}

async fn kick_blocked_user<
    T: serenity::CacheHttp + AsRef<serenity::Http> + AsRef<serenity::Cache> + Copy,
>(
    ctx: T,
    guild: serenity::GuildId,
    user: serenity::UserId,
) -> Result<(), Error> {
    let dm = user.create_dm_channel(ctx).await?;
    // TODO: Get invite
    dm.say(ctx, format!("{}, you have been kicked from {} for having a blocked image in your profile picture. Please change your profile and reapply.", user.mention(), guild.name(ctx).unwrap_or(String::from("the server")))).await?;

    guild
        .kick_with_reason(ctx, user, "Blocked image in profile picture")
        .await?;
    Ok(())
}

async fn get_response(
    http: std::sync::Arc<serenity::Http>,
    interaction: serenity::CollectComponentInteraction,
) -> Option<(usize, bool)> {
    if let Some(response) = interaction.await {
        let mut split_string = response.data.custom_id.split('-');
        let index = split_string.next().and_then(|x| x.parse::<usize>().ok());
        let result = split_string.next().and_then(|x| match x {
            "keep" => Some(false),
            "block" => Some(true),
            _ => None,
        });
        response.defer(http).await.ok();
        return index.and_then(|a| result.map(|b| (a, b)));
    }
    None
}
