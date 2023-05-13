use std::borrow::Cow;

use super::ContainBytes;
use super::{t, Context, Error};
use crate::{
    check_mod_role,
    entities::{prelude::*, *},
};
use itertools::Itertools;
use poise::serenity_prelude as serenity;
use sea_orm::*;
use serenity::utils::parse_role;
use serenity::Mentionable;
use tracing::instrument;

#[derive(FromQueryResult)]
struct AcceptUserServerData {
    questioning_category: i64,
    questioning_role: i64,
    mod_channel: i64,
    main_channel: i64,
    member_role: i64,
    mod_role: i64,
}

#[derive(FromQueryResult)]
struct QuestionUserServerData {
    questioning_category: i64,
    questioning_role: i64,
    member_role: i64,
    mod_role: i64,
}

#[instrument(skip_all, err)]
pub async fn alert_new_user(
    member: &serenity::Member,
    guild: serenity::GuildId,
    reference: super::EventReference<'_>,
) -> Result<(), super::Error> {
    super::mod_log(
        reference.0,
        reference.3,
        guild,
        None,
        format!("User {} joined", member.mention()),
    )
    .await?;
    Ok(())
}

/// Lets a user into the server proper and sends a welcome message
#[instrument(skip_all, err)]
#[poise::command(slash_command, context_menu_command = "Accept User", guild_only)]
pub async fn accept(ctx: Context<'_>, user: serenity::User) -> Result<(), Error> {
    let guild = ctx
        .guild_id()
        .ok_or(super::FedBotError::new("command called outside server"))?;

    let server_data: AcceptUserServerData = Servers::find_by_id(guild.as_u64().repack())
        .select_only()
        .column(servers::Column::Id)
        .column(servers::Column::QuestioningCategory)
        .column(servers::Column::QuestioningRole)
        .column(servers::Column::ModChannel)
        .column(servers::Column::MainChannel)
        .column(servers::Column::MemberRole)
        .column(servers::Column::ModRole)
        .into_model()
        .one(&ctx.data().db)
        .await?
        .ok_or(super::FedBotError::new("Failed to find query"))?;
    let (questioning_category, questioning_role, mod_channel, main_channel, member_role, mod_role) = (
        serenity::ChannelId(server_data.questioning_category.repack()),
        serenity::RoleId(server_data.questioning_role.repack()),
        serenity::ChannelId(server_data.mod_channel.repack()),
        serenity::ChannelId(server_data.main_channel.repack()),
        serenity::RoleId(server_data.member_role.repack()),
        serenity::RoleId(server_data.mod_role.repack()),
    );

    check_mod_role!(ctx, guild, mod_role);

    crate::defer!(ctx);

    if user.has_role(ctx, guild, member_role).await? {
        ctx.send(|f| {
            f.content("User already is accepted!")
                .ephemeral(ctx.data().is_ephemeral)
        })
        .await?;
        return Ok(());
    }

    let mut member = guild.member(ctx, user.id).await?;
    member.add_role(ctx, member_role).await?;

    let guild_name = guild
        .name(ctx)
        .ok_or(super::FedBotError::new("cannot get guild name"))?;
    main_channel
        .send_message(ctx, |f| {
            f.content(format!(
                "Welcome to {}, {}. Everyone say hi!",
                guild_name,
                user.mention()
            ))
        })
        .await?;

    let mut send_response = true;
    if user.has_role(ctx, guild, questioning_role).await? {
        member.remove_role(ctx, questioning_role).await?;
        if let Some(channel) = guild.channels(ctx).await?.into_values().find(|x| {
            x.parent_id == Some(questioning_category)
                && x.name.ends_with(&format!("-{}", member.user.id))
        }) {
            if channel.id == ctx.channel_id() {
                send_response = false;
            }
            clear_questioning(
                ctx,
                questioning_category,
                mod_channel,
                Some(member),
                channel,
            )
            .await?;
        } else {
            return Err(super::FedBotError::new("questioning channel not found").into());
        }
    }

    super::mod_log(
        ctx.serenity_context(),
        ctx.data(),
        guild,
        None,
        format!(
            "User {} accepted by mod {}",
            user.id.mention(),
            ctx.author().mention()
        ),
    )
    .await?;
    if send_response {
        ctx.send(|f| {
            f.content("Accepted user!")
                .ephemeral(ctx.data().is_ephemeral)
        })
        .await?;
    }
    Ok(())
}

struct LoggedMessage {
    filenames: Vec<String>,
    content: String,
    timestamp: serenity::Timestamp,
    author: (String, String, String),
}

const MAX_TOTAL_EMBED_LENGTH: usize = 6000;
const MAX_EMBEDS_PER_MESSAGE: usize = 5;

#[instrument(skip_all, err)]
#[poise::command(slash_command, guild_only)]
pub async fn purge_questioning(ctx: Context<'_>) -> Result<(), Error> {
    let guild = ctx
        .guild_id()
        .ok_or(super::FedBotError::new("command called outside server"))?;

    let server_data: AcceptUserServerData = Servers::find_by_id(guild.as_u64().repack())
        .select_only()
        .column(servers::Column::Id)
        .column(servers::Column::QuestioningCategory)
        .column(servers::Column::QuestioningRole)
        .column(servers::Column::ModChannel)
        .column(servers::Column::MainChannel)
        .column(servers::Column::MemberRole)
        .column(servers::Column::ModRole)
        .into_model()
        .one(&ctx.data().db)
        .await?
        .ok_or(super::FedBotError::new("Failed to find query"))?;
    let (questioning_category, mod_channel, mod_role) = (
        serenity::ChannelId(server_data.questioning_category.repack()),
        serenity::ChannelId(server_data.mod_channel.repack()),
        serenity::RoleId(server_data.mod_role.repack()),
    );

    check_mod_role!(ctx, guild, mod_role);

    crate::defer!(ctx);

    if let serenity::Channel::Guild(x) = ctx.channel_id().to_channel(ctx).await? {
        clear_questioning(ctx, questioning_category, mod_channel, None, x).await?;
    } else {
        return Err(super::FedBotError::new("channel is not a guild channel").into());
    }

    Ok(())
}

#[allow(clippy::too_many_lines)]
async fn clear_questioning(
    ctx: Context<'_>,
    questioning_category: serenity::ChannelId,
    questioning_log_channel: serenity::ChannelId,
    member: Option<serenity::Member>,
    channel: serenity::GuildChannel,
) -> Result<(), Error> {
    let mut messages = channel.messages(ctx, |f| f).await?;

    if let Some(mut member) = member {
        if let Some(i) = messages
            .iter()
            .find(|x| x.author.id == ctx.framework().bot_id)
        {
            if let Some(embed) = i.embeds.get(0) {
                if embed.title == Some("Roles".to_owned()) {
                    if let Some(roles) = embed.description.as_ref().map(|x| {
                        x.split(' ')
                            .filter_map(parse_role)
                            .map(serenity::RoleId)
                            .collect::<Vec<_>>()
                    }) {
                        if !roles.is_empty() {
                            member.add_roles(ctx, roles.as_slice()).await?;
                        }
                    }
                }
            }
        }

        channel
            .create_permission(
                ctx,
                &serenity::PermissionOverwrite {
                    allow: serenity::Permissions::empty(),
                    deny: serenity::Permissions::VIEW_CHANNEL,
                    kind: serenity::PermissionOverwriteType::Member(member.user.id),
                },
            )
            .await?;
    }

    messages.reverse();
    let first_message = messages
        .first()
        .ok_or(super::FedBotError::new("cannot get first message"))?;
    let start_time = first_message.timestamp.unix_timestamp();
    let questioned_user = serenity::UserId(
        super::USER
            .captures(first_message.content.as_str())
            .ok_or(super::FedBotError::new("cannot get user in question(ing)"))?
            .get(1)
            .ok_or(super::FedBotError::new("malformed regex"))?
            .as_str()
            .parse()?,
    )
    .to_user(ctx)
    .await?;

    let log_thread = questioning_log_channel
        .create_public_thread(
            ctx,
            questioning_log_channel
                .send_message(ctx, |f| {
                    f.content(format!(
                        "Log from {} channel with {} on <t:{}:f>",
                        questioning_category.mention(),
                        questioned_user.mention(),
                        start_time
                    ))
                })
                .await?
                .id,
            |f| {
                f.name(format!(
                    "{}{}-{}-{}",
                    &questioned_user.name,
                    questioned_user.discriminator,
                    questioned_user.id,
                    start_time
                ))
            },
        )
        .await?;

    let mut messages_vec = vec![];
    let mut attachments_vec = vec![];
    let mut total_length = 0;

    for i in messages {
        if total_length > MAX_TOTAL_EMBED_LENGTH || messages_vec.len() > MAX_EMBEDS_PER_MESSAGE {
            send_logged_messages(ctx, log_thread.id, attachments_vec, messages_vec).await?;
            attachments_vec = vec![];
            messages_vec = vec![];
            total_length = 0;
        }

        for j in &i.attachments {
            if let Ok(x) = t(ctx.data().reqwest.get(&j.url).send().await) {
                if let Ok(y) = t(x.bytes().await) {
                    attachments_vec.push(serenity::AttachmentType::Bytes {
                        data: Cow::Owned(y.to_vec()),
                        filename: j.filename.clone(),
                    });
                }
            }
        }

        let this_message = LoggedMessage {
            filenames: i.attachments.into_iter().map(|x| x.filename).collect(),
            content: i.content,
            timestamp: i.timestamp,
            author: (
                i.author.face(),
                i.author.tag(),
                format!("https://discordapp.com/users/{}", i.author.id),
            ),
        };

        total_length += this_message.content.len()
            + this_message.author.0.len()
            + this_message.author.1.len()
            + this_message.author.2.len();
        messages_vec.push(this_message);
    }
    if !messages_vec.is_empty() {
        send_logged_messages(ctx, log_thread.id, attachments_vec, messages_vec).await?;
    }
    channel.delete(ctx).await?;

    Ok(())
}

async fn send_logged_messages(
    ctx: Context<'_>,
    log_thread: serenity::ChannelId,
    attachments: Vec<serenity::AttachmentType<'_>>,
    messages: Vec<LoggedMessage>,
) -> Result<(), Error> {
    log_thread
        .send_files(ctx, attachments, |f| {
            for i in messages {
                f.add_embed(|f| {
                    f.author(|x| x.icon_url(i.author.0).name(i.author.1).url(i.author.2));
                    for j in i.filenames {
                        f.attachment(j);
                    }
                    f.description(i.content).timestamp(i.timestamp)
                });
            }
            f.allowed_mentions(|f| f.empty_users())
        })
        .await?;
    Ok(())
}

/// Lets a user back into the server proper from questioning
#[instrument(skip_all, err)]
#[poise::command(
    slash_command,
    context_menu_command = "Return User",
    guild_only,
    rename = "return"
)]
pub async fn return_(ctx: Context<'_>, user: serenity::User) -> Result<(), Error> {
    let guild = ctx
        .guild_id()
        .ok_or(super::FedBotError::new("command called outside server"))?;

    let server_data: AcceptUserServerData = Servers::find_by_id(guild.as_u64().repack())
        .select_only()
        .column(servers::Column::Id)
        .column(servers::Column::QuestioningCategory)
        .column(servers::Column::QuestioningRole)
        .column(servers::Column::ModChannel)
        .column(servers::Column::MainChannel)
        .column(servers::Column::MemberRole)
        .column(servers::Column::ModRole)
        .into_model()
        .one(&ctx.data().db)
        .await?
        .ok_or(super::FedBotError::new("Failed to find query"))?;
    let (questioning_category, questioning_role, mod_channel, member_role, mod_role) = (
        serenity::ChannelId(server_data.questioning_category.repack()),
        serenity::RoleId(server_data.questioning_role.repack()),
        serenity::ChannelId(server_data.mod_channel.repack()),
        serenity::RoleId(server_data.member_role.repack()),
        serenity::RoleId(server_data.mod_role.repack()),
    );

    check_mod_role!(ctx, guild, mod_role);

    crate::defer!(ctx);

    if user.has_role(ctx, guild, member_role).await?
        & !user.has_role(ctx, guild, questioning_role).await?
    {
        ctx.send(|f| {
            f.content("User is not in questioning!")
                .ephemeral(ctx.data().is_ephemeral)
        })
        .await?;
        return Ok(());
    }

    let mut member = guild.member(ctx, user.id).await?;
    member.add_role(ctx, member_role).await?;
    member.remove_role(ctx, questioning_role).await?;

    let mut send_response = true;
    if let Some(channel) = guild.channels(ctx).await?.into_values().find(|x| {
        x.parent_id == Some(questioning_category)
            && x.name.ends_with(&format!("-{}", member.user.id))
    }) {
        if channel.id == ctx.channel_id() {
            send_response = false;
        }
        clear_questioning(
            ctx,
            questioning_category,
            mod_channel,
            Some(member),
            channel,
        )
        .await?;
    } else {
        return Err(super::FedBotError::new("questioning channel not found").into());
    }

    super::mod_log(
        ctx.serenity_context(),
        ctx.data(),
        guild,
        None,
        format!(
            "User {} returned from questioning by mod {}",
            user.mention(),
            ctx.author().mention()
        ),
    )
    .await?;
    if send_response {
        ctx.send(|f| {
            f.content("Returned user!")
                .ephemeral(ctx.data().is_ephemeral)
        })
        .await?;
    }
    Ok(())
}

/// Send a user to questioning and optionally send a warning/explanation message
#[instrument(skip_all, err)]
#[poise::command(slash_command, context_menu_command = "Question User", guild_only)]
pub async fn question(ctx: Context<'_>, user: serenity::User) -> Result<(), Error> {
    let guild = ctx
        .guild_id()
        .ok_or(super::FedBotError::new("command called outside server"))?;

    let server_data: QuestionUserServerData = Servers::find_by_id(guild.as_u64().repack())
        .select_only()
        .column(servers::Column::Id)
        .column(servers::Column::QuestioningCategory)
        .column(servers::Column::QuestioningRole)
        .column(servers::Column::ModChannel)
        .column(servers::Column::MemberRole)
        .column(servers::Column::ModRole)
        .into_model()
        .one(&ctx.data().db)
        .await?
        .ok_or(super::FedBotError::new("Failed to find query"))?;
    let (questioning_category, questioning_role, member_role, mod_role) = (
        serenity::ChannelId(server_data.questioning_category.repack()),
        serenity::RoleId(server_data.questioning_role.repack()),
        serenity::RoleId(server_data.member_role.repack()),
        serenity::RoleId(server_data.mod_role.repack()),
    );

    check_mod_role!(ctx, guild, mod_role);

    crate::defer!(ctx);

    if user.has_role(ctx, guild, questioning_role).await? {
        ctx.send(|f| {
            f.content("User is already in questioning!")
                .ephemeral(ctx.data().is_ephemeral)
        })
        .await?;
        return Ok(());
    }

    let mut member = guild.member(ctx, user.id).await?;
    member.remove_role(ctx, member_role).await?;

    let roles = member.roles.clone();

    let questioning_channel: serenity::GuildChannel;

    if let Some(channel) = guild.channels(ctx).await?.into_values().find(|x| {
        x.parent_id == Some(questioning_category) && x.name.ends_with(&format!("-{}", user.id))
    }) {
        questioning_channel = channel;
    } else {
        questioning_channel = guild
            .create_channel(ctx, |f| {
                f.category(questioning_category)
                    .kind(serenity::ChannelType::Text)
                    .name(format!("{}{}-{}", user.name, user.discriminator, user.id))
            })
            .await?;
    }

    questioning_channel
        .create_permission(
            ctx,
            &serenity::PermissionOverwrite {
                allow: serenity::Permissions::VIEW_CHANNEL,
                deny: serenity::Permissions::empty(),
                kind: serenity::PermissionOverwriteType::Member(user.id),
            },
        )
        .await?;

    questioning_channel
        .create_permission(
            ctx,
            &serenity::PermissionOverwrite {
                allow: serenity::Permissions::VIEW_CHANNEL,
                deny: serenity::Permissions::empty(),
                kind: serenity::PermissionOverwriteType::Role(mod_role),
            },
        )
        .await?;

    let default_role = serenity::RoleId(guild.0); // @everyone has the same id as the guild
    questioning_channel
        .create_permission(
            ctx,
            &serenity::PermissionOverwrite {
                allow: serenity::Permissions::empty(),
                deny: serenity::Permissions::VIEW_CHANNEL,
                kind: serenity::PermissionOverwriteType::Role(default_role),
            },
        )
        .await?;

    questioning_channel
        .send_message(ctx, |f| {
            f.content(format!(
                "{}, you have been sent to questioning by mod {}.",
                user.mention(),
                ctx.author().mention()
            ))
            .add_embed(|f| {
                f.title("Roles")
                    .author(|f| f.icon_url(member.face()).name(member.user.tag()))
                    .description(roles.iter().map(Mentionable::mention).format(" "))
            })
        })
        .await?;

    member.remove_roles(ctx, &roles).await?;
    member.add_role(ctx, questioning_role).await?;

    super::mod_log(
        ctx.serenity_context(),
        ctx.data(),
        guild,
        None,
        format!(
            "User {} sent to questioning by mod {}",
            user.mention(),
            ctx.author().mention()
        ),
    )
    .await?;
    ctx.send(|f| {
        f.content("Sent user to questioning!")
            .ephemeral(ctx.data().is_ephemeral)
    })
    .await?;
    Ok(())
}
