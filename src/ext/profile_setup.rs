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

use super::ContainBytes;
use super::{entry_modal, Context, Error};
use crate::{
    check_admin,
    entities::{prelude::*, *},
};
use poise::serenity_prelude as serenity;
use sea_orm::*;
use tracing::instrument;

mod channel_overrides {
    use super::*;

    pub async fn mod_channel(
        ctx: Context<'_>,
        x: serenity::ChannelId,
        default_role: serenity::RoleId,
        mod_role: serenity::RoleId,
    ) -> Result<(), Error> {
        x.create_permission(
            ctx,
            &serenity::PermissionOverwrite {
                allow: serenity::Permissions::VIEW_CHANNEL,
                deny: serenity::Permissions::empty(),
                kind: serenity::PermissionOverwriteType::Role(mod_role),
            },
        )
        .await?;
        x.create_permission(
            ctx,
            &serenity::PermissionOverwrite {
                allow: serenity::Permissions::empty(),
                deny: serenity::Permissions::VIEW_CHANNEL,
                kind: serenity::PermissionOverwriteType::Role(default_role),
            },
        )
        .await?;
        Ok(())
    }

    pub async fn rules_channel(
        ctx: Context<'_>,
        x: serenity::ChannelId,
        default_role: serenity::RoleId,
    ) -> Result<(), Error> {
        x.create_permission(
            ctx,
            &serenity::PermissionOverwrite {
                allow: serenity::Permissions::VIEW_CHANNEL,
                deny: serenity::Permissions::SEND_MESSAGES,
                kind: serenity::PermissionOverwriteType::Role(default_role),
            },
        )
        .await?;
        Ok(())
    }

    pub async fn screening_channel(
        ctx: Context<'_>,
        x: serenity::ChannelId,
        default_role: serenity::RoleId,
        mod_role: serenity::RoleId,
        member_role: serenity::RoleId,
        questioning_role: serenity::RoleId,
    ) -> Result<(), Error> {
        x.create_permission(
            ctx,
            &serenity::PermissionOverwrite {
                allow: serenity::Permissions::VIEW_CHANNEL,
                deny: serenity::Permissions::SEND_MESSAGES,
                kind: serenity::PermissionOverwriteType::Role(default_role),
            },
        )
        .await?;
        x.create_permission(
            ctx,
            &serenity::PermissionOverwrite {
                allow: serenity::Permissions::VIEW_CHANNEL,
                deny: serenity::Permissions::SEND_MESSAGES,
                kind: serenity::PermissionOverwriteType::Role(mod_role),
            },
        )
        .await?;
        x.create_permission(
            ctx,
            &serenity::PermissionOverwrite {
                allow: serenity::Permissions::empty(),
                deny: serenity::Permissions::VIEW_CHANNEL,
                kind: serenity::PermissionOverwriteType::Role(member_role),
            },
        )
        .await?;
        x.create_permission(
            ctx,
            &serenity::PermissionOverwrite {
                allow: serenity::Permissions::empty(),
                deny: serenity::Permissions::VIEW_CHANNEL,
                kind: serenity::PermissionOverwriteType::Role(questioning_role),
            },
        )
        .await?;
        Ok(())
    }

    pub async fn questioning_category(
        ctx: Context<'_>,
        x: serenity::ChannelId,
        default_role: serenity::RoleId,
        questioning_role: serenity::RoleId,
        mod_role: serenity::RoleId,
    ) -> Result<(), Error> {
        x.create_permission(
            ctx,
            &serenity::PermissionOverwrite {
                allow: serenity::Permissions::empty(),
                deny: serenity::Permissions::VIEW_CHANNEL,
                kind: serenity::PermissionOverwriteType::Role(default_role),
            },
        )
        .await?;
        x.create_permission(
            ctx,
            &serenity::PermissionOverwrite {
                allow: serenity::Permissions::SEND_MESSAGES,
                deny: serenity::Permissions::VIEW_CHANNEL,
                kind: serenity::PermissionOverwriteType::Role(questioning_role),
            },
        )
        .await?;
        x.create_permission(
            ctx,
            &serenity::PermissionOverwrite {
                allow: serenity::Permissions::SEND_MESSAGES | serenity::Permissions::VIEW_CHANNEL,
                deny: serenity::Permissions::empty(),
                kind: serenity::PermissionOverwriteType::Role(mod_role),
            },
        )
        .await?;
        Ok(())
    }
}

/// Blank supercommand
#[instrument(skip_all, err)]
#[poise::command(
    slash_command,
    subcommands("init", "update", "entry_modal::set_entry_modal"),
    guild_only
)]
pub async fn profile(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// Create a new server profile
#[instrument(skip_all, err)]
#[poise::command(slash_command, guild_only)]
#[allow(clippy::too_many_arguments)]
async fn init(
    ctx: Context<'_>,
    #[channel_types("Text")] rules_channel: serenity::GuildChannel,
    #[channel_types("Text")] screening_channel: serenity::GuildChannel,
    questioning_role: serenity::Role,
    #[channel_types("Category")] questioning_category: serenity::Channel,
    mod_role: serenity::Role,
    #[channel_types("Text")] mod_channel: serenity::GuildChannel,
    member_role: serenity::Role,
    #[channel_types("Text")] main_channel: serenity::GuildChannel,
) -> Result<(), Error> {
    let guild = ctx
        .guild_id()
        .ok_or(super::FedBotError::new("command called outside server"))?;

    check_admin!(ctx, guild);

    let maybe_category = questioning_category;
    let questioning_category: serenity::ChannelCategory;
    if let serenity::Channel::Category(x) = maybe_category {
        questioning_category = x;
    } else {
        return Err(super::FedBotError::new("questioning_category is not a category").into());
    }

    crate::defer!(ctx);

    let new_server = servers::ActiveModel {
        id: ActiveValue::Set(guild.as_u64().repack()),
        rules_channel: ActiveValue::Set(rules_channel.id.as_u64().repack()),
        screening_channel: ActiveValue::Set(screening_channel.id.as_u64().repack()),
        questioning_role: ActiveValue::Set(questioning_role.id.as_u64().repack()),
        questioning_category: ActiveValue::Set(questioning_category.id.as_u64().repack()),
        mod_role: ActiveValue::Set(mod_role.id.as_u64().repack()),
        mod_channel: ActiveValue::Set(mod_channel.id.as_u64().repack()),
        member_role: ActiveValue::Set(member_role.id.as_u64().repack()),
        main_channel: ActiveValue::Set(main_channel.id.as_u64().repack()),
        ..Default::default()
    };
    Servers::insert(new_server).exec(&ctx.data().db).await?;

    let default_role = serenity::RoleId(guild.0); // @everyone has the same id as the guild
    let default_perms = if let Some(x) = default_role.to_role_cached(ctx) {
        x
    } else {
        guild
            .roles(ctx)
            .await?
            .remove(&default_role)
            .ok_or(super::FedBotError::new("role missing from guild"))?
    }
    .permissions;
    guild
        .edit_role(ctx, default_role, |f| {
            f.permissions(default_perms & !serenity::Permissions::VIEW_CHANNEL)
        })
        .await?;

    guild
        .edit_role(ctx, member_role.id, |f| {
            f.permissions(member_role.permissions | serenity::Permissions::VIEW_CHANNEL)
        })
        .await?;

    channel_overrides::mod_channel(ctx, mod_channel.id, default_role, mod_role.id).await?;
    channel_overrides::rules_channel(ctx, rules_channel.id, default_role).await?;
    channel_overrides::screening_channel(
        ctx,
        screening_channel.id,
        default_role,
        mod_role.id,
        member_role.id,
        questioning_role.id,
    )
    .await?;
    channel_overrides::questioning_category(
        ctx,
        questioning_category.id,
        default_role,
        questioning_role.id,
        mod_role.id,
    )
    .await?;

    super::entry_modal::display_entry_modal(ctx.serenity_context(), ctx.data(), guild).await?;

    ctx.send(|f| {
        f.content("Created server profile!")
            .ephemeral(ctx.data().is_ephemeral)
    })
    .await
    .map(|_| ())
    .map_err(Into::into)
}

#[derive(FromQueryResult)]
struct UpdateServerData {
    questioning_role: i64,
    member_role: i64,
    mod_role: i64,
}

/// Update an existing server profile
#[instrument(skip_all, err)]
#[poise::command(slash_command, guild_only)]
#[allow(clippy::too_many_arguments)]
async fn update(
    ctx: Context<'_>,
    #[channel_types("Text")] rules_channel: Option<serenity::GuildChannel>,
    #[channel_types("Text")] screening_channel: Option<serenity::GuildChannel>,
    questioning_role: Option<serenity::Role>,
    #[channel_types("Category")] questioning_category: Option<serenity::Channel>,
    mod_role: Option<serenity::Role>,
    #[channel_types("Text")] mod_channel: Option<serenity::GuildChannel>,
    member_role: Option<serenity::Role>,
    #[channel_types("Text")] main_channel: Option<serenity::GuildChannel>,
) -> Result<(), Error> {
    let guild = ctx
        .guild_id()
        .ok_or(super::FedBotError::new("command called outside server"))?;

    check_admin!(ctx, guild);

    let new_server = servers::ActiveModel {
        id: ActiveValue::Unchanged(guild.as_u64().repack()),
        rules_channel: if let Some(x) = &rules_channel {
            ActiveValue::Set(x.id.as_u64().repack())
        } else {
            ActiveValue::NotSet
        },
        screening_channel: if let Some(x) = &screening_channel {
            ActiveValue::Set(x.id.as_u64().repack())
        } else {
            ActiveValue::NotSet
        },
        questioning_role: if let Some(x) = &questioning_role {
            ActiveValue::Set(x.id.as_u64().repack())
        } else {
            ActiveValue::NotSet
        },
        questioning_category: if let Some(x) = &questioning_category {
            ActiveValue::Set(x.id().as_u64().repack())
        } else {
            ActiveValue::NotSet
        },
        mod_role: if let Some(x) = &mod_role {
            ActiveValue::Set(x.id.as_u64().repack())
        } else {
            ActiveValue::NotSet
        },
        mod_channel: if let Some(x) = &mod_channel {
            ActiveValue::Set(x.id.as_u64().repack())
        } else {
            ActiveValue::NotSet
        },
        member_role: if let Some(x) = &member_role {
            ActiveValue::Set(x.id.as_u64().repack())
        } else {
            ActiveValue::NotSet
        },
        main_channel: if let Some(x) = &main_channel {
            ActiveValue::Set(x.id.as_u64().repack())
        } else {
            ActiveValue::NotSet
        },
        ..Default::default()
    };
    Servers::update(new_server).exec(&ctx.data().db).await?;

    if let Some(x) = member_role {
        guild
            .edit_role(ctx, x.id, |f| {
                f.permissions(x.permissions | serenity::Permissions::VIEW_CHANNEL)
            })
            .await?;
    }

    let server_data: UpdateServerData = Servers::find_by_id(guild.as_u64().repack())
        .select_only()
        .column(servers::Column::Id)
        .column(servers::Column::QuestioningRole)
        .column(servers::Column::MemberRole)
        .column(servers::Column::ModRole)
        .into_model()
        .one(&ctx.data().db)
        .await?
        .ok_or(super::FedBotError::new("Failed to find query"))?;
    let (questioning_role, member_role, mod_role) = (
        serenity::RoleId(server_data.questioning_role.repack()),
        serenity::RoleId(server_data.member_role.repack()),
        serenity::RoleId(server_data.mod_role.repack()),
    );

    let default_role = serenity::RoleId(guild.0); // @everyone has the same id as the guild
    let default_perms = if let Some(x) = default_role.to_role_cached(ctx) {
        x
    } else {
        guild
            .roles(ctx)
            .await?
            .remove(&default_role)
            .ok_or(super::FedBotError::new("role missing from guild"))?
    }
    .permissions;
    guild
        .edit_role(ctx, default_role, |f| {
            f.permissions(default_perms & !serenity::Permissions::VIEW_CHANNEL)
        })
        .await?;

    if let Some(x) = mod_channel {
        channel_overrides::mod_channel(ctx, x.id, default_role, mod_role).await?;
    }
    if let Some(x) = rules_channel {
        channel_overrides::rules_channel(ctx, x.id, default_role).await?;
    }
    if let Some(x) = screening_channel {
        channel_overrides::screening_channel(
            ctx,
            x.id,
            default_role,
            mod_role,
            member_role,
            questioning_role,
        )
        .await?;

        super::entry_modal::display_entry_modal(ctx.serenity_context(), ctx.data(), guild).await?;
    }
    if let Some(maybe_category) = questioning_category {
        let x: serenity::ChannelCategory;
        if let serenity::Channel::Category(y) = maybe_category {
            x = y;
        } else {
            return Err(super::FedBotError::new("questioning_category is not a category").into());
        }

        channel_overrides::questioning_category(
            ctx,
            x.id,
            default_role,
            questioning_role,
            mod_role,
        )
        .await?;
    }

    ctx.send(|f| {
        f.content("Updated server profile!")
            .ephemeral(ctx.data().is_ephemeral)
    })
    .await
    .map(|_| ())
    .map_err(Into::into)
}
