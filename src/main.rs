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

#![feature(
    async_closure,
    is_some_and,
    fs_try_exists,
    path_file_prefix,
    iter_array_chunks,
    hash_drain_filter
)]
#![allow(clippy::wildcard_imports)]

use dunce::canonicalize;
use entities::prelude::*;
use ext::TriggerCooldown;
use http_cache_reqwest::{CACacheManager, Cache, CacheMode, HttpCache};
use poise::serenity_prelude as serenity;
use poise::Event;
use poise::PrefixFrameworkOptions;
use reqwest::Client;
use reqwest_middleware::ClientBuilder;
use sea_orm::*;
use tokio::sync::RwLock;
use tracing::{error, instrument, log::LevelFilter, Level};
use tracing_appender::rolling::{RollingFileAppender, Rotation};

use std::collections::HashMap;
use std::fs;
use std::{boxed::Box, path::Path};

mod entities;
mod ext;
use self::ext::{
    get_alert_channel, t, Data, Error, EventReference, FedBotError, FrameworkContext,
    FrameworkError,
};

const EPHEMERAL_MESSAGES: bool = true;
const DB_FILE: &str = "test.db";
const DB_MEM_PAGES: isize = 12_500; // Pages are normally 4096 bytes each

#[instrument(skip_all, err)]
async fn dispatch_events<'a>(
    ctx: &'a serenity::Context,
    event: &'a Event<'a>,
    system: FrameworkContext<'a>,
    data: &'a Data,
) -> Result<(), Error> {
    let reference = (ctx, event, system, data);
    match event {
        Event::Message { new_message } => {
            if !new_message.is_own(ctx) {
                if let Some(guild) = new_message.guild_id {
                    let _ = ext::profanity_checks::filter_message(
                        new_message,
                        new_message.channel_id,
                        new_message.id,
                        &new_message.author,
                        reference,
                    )
                    .await?
                        || ext::image_filtering::filter_message(
                            new_message,
                            guild,
                            new_message.channel_id,
                            new_message.id,
                            &new_message.author,
                            reference,
                        )
                        .await?
                        || ext::triggers::fire_triggers(new_message, guild, reference).await?;
                }
            }
        }
        Event::MessageUpdate { event, .. } => {
            // Message event may be partial so we may have to ask for more info
            let author: &serenity::User;
            let author_guard: serenity::User;
            if let Some(user) = event.author.as_ref() {
                author = user;
            } else {
                author_guard = event.channel_id.message(ctx, event.id).await?.author;
                author = &author_guard;
            }

            if author.id != ctx.cache.current_user_id() {
                if let Some(guild) = event.guild_id {
                    let _ = ext::profanity_checks::filter_message(
                        event,
                        event.channel_id,
                        event.id,
                        author,
                        reference,
                    )
                    .await?
                        || ext::image_filtering::filter_message(
                            event,
                            guild,
                            event.channel_id,
                            event.id,
                            author,
                            reference,
                        )
                        .await?;
                }
            }
        }
        Event::GuildStickersUpdate {
            guild_id,
            current_state,
        } => {
            ext::image_filtering::filter_stickers(
                current_state
                    .clone()
                    .into_values()
                    .collect::<Vec<serenity::Sticker>>(),
                *guild_id,
                reference,
            )
            .await?;
        }
        Event::GuildEmojisUpdate {
            guild_id,
            current_state,
        } => {
            ext::image_filtering::filter_emojis(
                current_state
                    .clone()
                    .into_values()
                    .collect::<Vec<serenity::Emoji>>(),
                *guild_id,
                reference,
            )
            .await?;
        }
        Event::GuildCreate { guild, is_new } => {
            prompt_guild_setup(guild, *is_new, reference).await?;
            // Fires on startup too
            ext::triggers::add_guild_triggers(guild, *is_new, reference).await?;
            if !*is_new {
                ext::entry_modal::display_entry_modal(reference.0, reference.3, guild.id).await?;
            }
        }
        Event::GuildMemberAddition { new_member } => {
            ext::user_screening::alert_new_user(new_member, new_member.guild_id, reference).await?;
            ext::image_filtering::filter_member(new_member, new_member.guild_id, reference).await?;
        }
        Event::GuildMemberUpdate { new, .. } => {
            ext::image_filtering::filter_member(new, new.guild_id, reference).await?;
        }
        Event::GuildUpdate {
            new_but_incomplete, ..
        } => {
            ext::image_filtering::filter_server(
                new_but_incomplete,
                new_but_incomplete.id,
                reference,
            )
            .await?;
        }
        Event::Ready { .. } => {
            set_db_pragmas(reference).await?;
            tokio::spawn(clean_trigger_cooldowns(
                reference.3.trigger_cooldown.clone(),
            ));
        }
        Event::ReactionAdd { add_reaction } => {
            if let Some(guild) = add_reaction.guild_id {
                ext::image_filtering::filter_reaction(add_reaction, guild, reference).await?;
            }
        }
        _ => (),
    }
    Ok(())
}

const CLEANING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3600);

async fn clean_trigger_cooldowns(cooldown: TriggerCooldown) {
    loop {
        tokio::time::sleep(CLEANING_INTERVAL).await;
        cooldown.clean().await;
    }
}

#[instrument(skip_all, err)]
async fn prompt_guild_setup(
    guild: &serenity::Guild,
    is_new: bool,
    reference: EventReference<'_>,
) -> Result<(), ext::Error> {
    if !is_new {
        // Fires on bot startup too, which we don't want
        return Ok(());
    }

    get_alert_channel(guild, reference).await?.send_message(reference.0, |f| f.content(
        concat!(
        "Thank you for adding FedBot to your server!\n",
        "To set up FedBot, please run `/profiles init`. (NOTE: you must have Administrator permissions to run this command.)\n",
        "If you have any questions, use `/help`.\n",
        )
    )).await.map(|_| ()).map_err(Into::into)
}

#[instrument(skip_all, err)]
async fn set_db_pragmas(reference: EventReference<'_>) -> Result<(), ext::Error> {
    // Set cache size
    reference
        .3
        .db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            format!(r"PRAGMA cache_size={DB_MEM_PAGES}"),
        ))
        .await?;
    reference
        .3
        .db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            format!(r"PRAGMA default_cache_size={DB_MEM_PAGES}"),
        ))
        .await?;

    // Set EXCLUSIVE mode since we're the only program using the db file
    reference
        .3
        .db
        .query_one(Statement::from_string(
            DbBackend::Sqlite,
            r"PRAGMA locking_mode=EXCLUSIVE".to_owned(),
        ))
        .await?;

    Ok(())
}

#[instrument(skip_all)]
async fn on_error(err: FrameworkError<'_>) {
    error!("{}", &err);
    match err {
        FrameworkError::Command { ctx, .. } => {
            _ = t(ctx
                .send(|f| {
                    f.content("Sorry, an error occured.")
                        .ephemeral(ctx.data().is_ephemeral)
                })
                .await);
        }
        FrameworkError::EventHandler { error, .. } => {
            error!("{}", error);
        }
        FrameworkError::ArgumentParse { error, .. } => {
            error!("{}", error);
        }
        FrameworkError::Setup { error, .. } => {
            error!("{}", error);
        }
        _ => (),
    }
}

#[tokio::main]
#[instrument(skip_all, err)]
async fn main() -> Result<(), Error> {
    let exe_path = canonicalize(Path::new(&std::env::current_exe()?))?;
    ext::profanity_checks::init_statics();

    let (non_blocking, _guard) = tracing_appender::non_blocking(RollingFileAppender::new(
        Rotation::NEVER,
        exe_path
            .parent()
            .ok_or(FedBotError::new("cannot locate exe folder"))?,
        format!(
            "{}.log",
            exe_path
                .file_prefix()
                .ok_or(FedBotError::new("cannot get exe stem"))?
                .to_str()
                .ok_or(FedBotError::new("cannot get exe stem"))?
        ),
    ));
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_writer(non_blocking)
        .with_ansi(false)
        .init();

    dotenv::from_path(&exe_path.with_file_name(".env"))?;

    let db_path = exe_path
        .with_file_name(DB_FILE)
        .as_os_str()
        .to_str()
        .ok_or(FedBotError::new("cannot locate exe file"))?
        .to_owned();

    let mut db_options = ConnectOptions::new(format!("sqlite://{}?mode=rwc", &db_path));
    db_options.sqlx_logging_level(LevelFilter::Debug);

    if !fs::try_exists(&db_path)? {
        let bootstrap_db = Database::connect(db_options.clone()).await?;
        // Add other tables as they are added to SCHEMA
        let tables = vec![DbBackend::Sqlite
            .build(&Schema::new(DbBackend::Sqlite).create_table_from_entity(Servers))];
        for i in tables {
            bootstrap_db.query_one(i).await?;
        }
        drop(bootstrap_db);
    }

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![
                ext::assorted::test(),
                ext::assorted::timestamp(),
                ext::assorted::purgeto(),
                ext::assorted::pirate_emoji(),
                ext::profile_setup::profile(),
                ext::user_screening::accept(),
                ext::user_screening::return_(),
                ext::user_screening::question(),
                ext::user_screening::purge_questioning(),
                ext::image_filtering::block_msg(),
                ext::image_filtering::block_pfp(),
                ext::image_filtering::block_server(),
                ext::assorted::move_(),
                ext::assorted::minesweeper(),
                ext::assorted::poll(),
                ext::assorted::invite(),
                ext::triggers::trigger(),
                ext::triggers::triggers(),
            ],
            event_handler: |ctx, event, system, data| {
                Box::pin(async move { dispatch_events(ctx, event, system, data).await })
            },
            on_error: |err| Box::pin(async move { on_error(err).await }),
            prefix_options: PrefixFrameworkOptions {
                prefix: None,
                ..Default::default()
            },
            ..Default::default()
        })
        .token(std::env::var("DISCORD_FEDBOT_TOKEN")?)
        .intents(serenity::GatewayIntents::all())
        .setup(|ctx, _ready, framework| {
            Box::pin(async move {
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                Ok(Data {
                    login_time: None,
                    is_ephemeral: EPHEMERAL_MESSAGES,
                    // users: HashMap::new(),
                    db: Database::connect(db_options).await?,
                    reqwest: ClientBuilder::new(Client::new())
                        .with(Cache(HttpCache {
                            mode: CacheMode::Default,
                            manager: CACacheManager::default(),
                            options: None,
                        }))
                        .build(),
                    hasher: image_hasher::HasherConfig::new()
                        .hash_size(ext::HASH_BYTES.into(), ext::HASH_BYTES.into())
                        .to_hasher(),
                    triggers: RwLock::new(HashMap::new()),
                    trigger_cooldown: TriggerCooldown::default(),
                })
            })
        });
    framework.run().await?;
    Ok(())
}
