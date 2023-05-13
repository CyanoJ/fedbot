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
use crate::{
    check_admin,
    entities::{prelude::*, *},
};
use itertools::Itertools;
use lazy_static::lazy_static;
use poise::serenity_prelude as serenity;
use poise::Modal;
use regex::Regex;
use sea_orm::*;
use std::collections::HashMap;
use tracing::{info, instrument};

lazy_static! {
    static ref TRIGGERS: Regex = Regex::new(r"(?:^|\s)!(\w+)").unwrap();
}

const MAX_TRIGGERS_PER_MESSAGE: usize = 4;

#[instrument(skip_all, err)]
pub async fn fire_triggers(
    message: &serenity::Message,
    guild: serenity::GuildId,
    reference: super::EventReference<'_>,
) -> Result<bool, super::Error> {
    if reference
        .3
        .trigger_cooldown
        .on_cooldown(message.author.id)
        .await
    {
        return Ok(false);
    }

    if let Some(triggers_map) = reference.3.triggers.read().await.get(&guild) {
        for i in TRIGGERS
            .captures_iter(&message.content)
            .take(MAX_TRIGGERS_PER_MESSAGE)
        {
            if let Some(trigger_text) = triggers_map.get(
                i.get(1)
                    .ok_or(super::FedBotError::new("malformed trigger"))?
                    .as_str()
                    .to_lowercase()
                    .as_str(),
            ) {
                message.reply(reference.0, trigger_text).await?;
            }
        }
    }
    reference
        .3
        .trigger_cooldown
        .activate(message.author.id)
        .await;
    Ok(false)
}

#[derive(FromQueryResult)]
struct GuildTriggers {
    triggers: Option<Vec<u8>>,
}

/// Get a list of all server triggers
#[instrument(skip_all, err)]
#[poise::command(slash_command, guild_only)]
pub async fn triggers(ctx: super::Context<'_>) -> Result<(), super::Error> {
    let guild = ctx
        .guild()
        .ok_or(super::FedBotError::new("command not in guild"))?
        .id;

    if let Some(triggers_map) = ctx.data().triggers.read().await.get(&guild) {
        let commands = triggers_map
            .keys()
            .map(|x| format!("!{x}"))
            .format("\n")
            .to_string();
        if !commands.is_empty() {
            ctx.send(|f| f.embed(|f| f.description(commands))).await?;
            return Ok(());
        }
    }

    ctx.send(|f| {
        f.content("No triggers in guild.")
            .ephemeral(ctx.data().is_ephemeral)
    })
    .await?;
    Ok(())
}

/// Blank supercommand
#[instrument(skip_all, err)]
#[poise::command(
    slash_command,
    subcommands("set_trigger", "remove_trigger"),
    guild_only
)]
pub async fn trigger(_ctx: super::Context<'_>) -> Result<(), super::Error> {
    Ok(())
}

fn check_trigger_name(name: &str) -> Result<bool, super::Error> {
    Ok(name
        == TRIGGERS
            .captures(&format!("!{name}"))
            .ok_or(super::FedBotError::new("malformed trigger"))?
            .get(1)
            .ok_or(super::FedBotError::new("malformed trigger"))?
            .as_str())
}

#[derive(Modal)]
#[name = "Add Value"]
struct TriggerValueModal {
    #[name = "Value"]
    #[paragraph]
    value: String,
}

/// Add/update a trigger
#[instrument(skip_all, err)]
#[poise::command(slash_command, guild_only, rename = "set")]
pub async fn set_trigger(
    ctx: super::Context<'_>,
    name: String,
    #[description = "Leave empty to use a modal for multiline text"] value: Option<String>,
) -> Result<(), super::Error> {
    let modal_ctx: super::ApplicationContext;
    if let super::Context::Application(inner_ctx) = ctx {
        modal_ctx = inner_ctx;
    } else {
        return Err(super::FedBotError::new("command must be used in application context").into());
    }

    let guild = ctx
        .guild()
        .ok_or(super::FedBotError::new("command not in guild"))?
        .id;

    check_admin!(ctx, guild);

    let value = if let Some(x) = value {
        x
    } else {
        TriggerValueModal::execute(modal_ctx)
            .await?
            .ok_or(super::FedBotError::new("no trigger value specified"))?
            .value
    };

    let name = name.to_lowercase();

    if !check_trigger_name(&name).unwrap_or(false) {
        ctx.send(|f| {
            f.content("Invalid trigger name.")
                .ephemeral(ctx.data().is_ephemeral)
        })
        .await?;
        return Ok(());
    }

    let raw_commands: GuildTriggers = Servers::find_by_id(guild.as_u64().repack())
        .select_only()
        .column(servers::Column::Id)
        .column(servers::Column::Triggers)
        .into_model()
        .one(&ctx.data().db)
        .await?
        .ok_or(super::FedBotError::new("Failed to find query"))?;

    info!(
        "User '{}#{}' added/updated trigger '{}'",
        ctx.author().name,
        ctx.author().discriminator,
        name.as_str()
    );

    let mut triggers = match raw_commands.triggers {
        Some(x) => rmp_serde::from_slice(&x)?,
        None => HashMap::new(),
    };
    triggers.insert(name.clone(), value.clone());

    let mut model: servers::ActiveModel = sea_orm::ActiveModelTrait::default();
    model.id = ActiveValue::Unchanged(guild.as_u64().repack());
    model.triggers = ActiveValue::Set(Some(rmp_serde::to_vec(&triggers)?));
    model.update(&ctx.data().db).await?;

    let mut mem_cache = ctx.data().triggers.write().await;
    if let Some(x) = mem_cache.get_mut(&guild) {
        x.insert(name, value);
    } else {
        let mut new_map = HashMap::new();
        new_map.insert(name, value);
        mem_cache.insert(guild, new_map);
    }
    drop(mem_cache);

    ctx.send(|f| {
        f.content("Added trigger!")
            .ephemeral(ctx.data().is_ephemeral)
    })
    .await?;

    Ok(())
}

/// Remove a trigger
#[instrument(skip_all, err)]
#[poise::command(slash_command, guild_only, rename = "remove")]
pub async fn remove_trigger(ctx: super::Context<'_>, name: String) -> Result<(), super::Error> {
    let guild = ctx
        .guild()
        .ok_or(super::FedBotError::new("command not in guild"))?
        .id;

    check_admin!(ctx, guild);

    let name = name.to_lowercase();

    if !check_trigger_name(&name).unwrap_or(false) {
        ctx.send(|f| {
            f.content("Invalid trigger name.")
                .ephemeral(ctx.data().is_ephemeral)
        })
        .await?;
        return Ok(());
    }

    let raw_commands: GuildTriggers = Servers::find_by_id(guild.as_u64().repack())
        .select_only()
        .column(servers::Column::Id)
        .column(servers::Column::Triggers)
        .into_model()
        .one(&ctx.data().db)
        .await?
        .ok_or(super::FedBotError::new("Failed to find query"))?;

    info!(
        "User '{}#{}' removed trigger '{}'",
        ctx.author().name,
        ctx.author().discriminator,
        name.as_str()
    );

    let mut triggers: HashMap<String, String> = match raw_commands.triggers {
        Some(x) => rmp_serde::from_slice(&x)?,
        None => return Err(super::FedBotError::new("no triggers to remove").into()),
    };

    triggers.remove(&name);

    let mut model: servers::ActiveModel = sea_orm::ActiveModelTrait::default();
    model.id = ActiveValue::Unchanged(guild.as_u64().repack());
    model.triggers = ActiveValue::Set(Some(rmp_serde::to_vec(&triggers)?));
    model.update(&ctx.data().db).await?;

    if let Some(x) = ctx.data().triggers.write().await.get_mut(&guild) {
        x.remove(&name);
    }

    ctx.send(|f| {
        f.content("Removed trigger!")
            .ephemeral(ctx.data().is_ephemeral)
    })
    .await?;

    Ok(())
}

#[instrument(skip_all, err)]
pub async fn add_guild_triggers(
    guild: &serenity::Guild,
    is_new: bool,
    reference: super::EventReference<'_>,
) -> Result<(), super::Error> {
    if is_new {
        return Ok(()); // For now
    }

    let raw_commands: GuildTriggers = Servers::find_by_id(guild.id.as_u64().repack())
        .select_only()
        .column(servers::Column::Id)
        .column(servers::Column::Triggers)
        .into_model()
        .one(&reference.3.db)
        .await?
        .ok_or(super::FedBotError::new("Failed to find query"))?;

    if let Some(trigger_binary) = raw_commands.triggers {
        reference
            .3
            .triggers
            .write()
            .await
            .insert(guild.id, rmp_serde::from_slice(&trigger_binary)?);
    }

    Ok(())
}
