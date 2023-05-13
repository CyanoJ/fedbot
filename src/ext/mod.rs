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

pub mod assorted;
pub mod entry_modal;
pub mod image_filtering;
pub mod profanity_checks;
pub mod profile_setup;
pub mod triggers;
pub mod user_screening;

use crate::entities::{prelude::*, *};
use lazy_static::lazy_static;
use poise::serenity_prelude as serenity;
use poise::Event;
use regex::Regex;
use reqwest_middleware::ClientWithMiddleware;
use sea_orm::DatabaseConnection;
use sea_orm::*;
use tokio::sync::RwLock;
use tracing::instrument;

use std::{collections::HashMap, error, fmt};

pub const HASH_BYTES: u8 = 8;

#[inline]
pub fn t<S, E: ToString + std::fmt::Display>(x: Result<S, E>) -> Result<S, E> {
    if let Err(err) = &x {
        tracing::error!("{}", err);
    }
    x
}

#[macro_export]
macro_rules! check_mod_role {
    ($ctx:expr, $guild:expr, $mod_role:expr) => {
        if !$ctx.author().has_role($ctx, $guild, $mod_role).await? {
            tracing::info!(
                "User '{}#{}' attempted to access privileged command '{}' in guild '{}'",
                $ctx.author().name,
                $ctx.author().discriminator,
                $ctx.invoked_command_name(),
                $guild
                    .name($ctx)
                    .ok_or($crate::ext::FedBotError::new("cannot get server name"))?
            );
            $ctx.send(|f| {
                f.ephemeral($ctx.data().is_ephemeral)
                    .content("You do not have authorization to access this command.")
            })
            .await?;
            return Ok(());
        }
    };
}

#[macro_export]
macro_rules! check_admin {
    ($ctx:expr, $guild:expr) => {
        if !$guild
            .member($ctx, $ctx.author().id)
            .await?
            .permissions($ctx)?
            .administrator()
        {
            tracing::info!(
                "User '{}#{}' attempted to access administrator command '{}' in guild '{}'",
                $ctx.author().name,
                $ctx.author().discriminator,
                $ctx.invoked_command_name(),
                $guild
                    .name($ctx)
                    .ok_or($crate::ext::FedBotError::new("cannot get server name"))?
            );
            $ctx.send(|f| {
                f.ephemeral($ctx.data().is_ephemeral).content(
                    "You do not have `ADMINISTRATOR` permissions and cannot access this command.",
                )
            })
            .await?;
            return Ok(());
        }
    };
}

#[macro_export]
macro_rules! defer {
    ($ctx:ident) => {
        if $ctx.data().is_ephemeral {
            $ctx.defer_ephemeral().await?;
        } else {
            $ctx.defer().await?;
        }
    };
}

lazy_static! {
    static ref EMOJI: Regex = Regex::new(r"<(a?):([\w_]+):(\d+)>").unwrap();
    static ref USER: Regex = Regex::new(r"<@(\d+)>").unwrap();
}

#[derive(Default, Clone)]
pub struct TriggerCooldown(
    std::sync::Arc<tokio::sync::RwLock<HashMap<serenity::UserId, std::time::Instant>>>,
);

pub struct Data {
    pub login_time: Option<serenity::Timestamp>,
    pub is_ephemeral: bool,
    // pub users: HashMap<serenity::UserId, AppUser, RandomState>,
    pub db: DatabaseConnection,
    pub hasher: image_hasher::Hasher,
    pub reqwest: ClientWithMiddleware,
    pub triggers: RwLock<HashMap<serenity::GuildId, HashMap<String, String>>>,
    pub trigger_cooldown: TriggerCooldown,
}

// User data, which is stored and accessible in all command invocations
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Context<'a> = poise::Context<'a, Data, Error>;
pub type ApplicationContext<'a> = poise::ApplicationContext<'a, Data, Error>;
pub type FrameworkContext<'a> = poise::FrameworkContext<'a, Data, Error>;
pub type FrameworkError<'a> = poise::FrameworkError<'a, Data, Error>;
// pub type Command = poise::Command<Data, Error>;

pub type EventReference<'a> = (
    &'a serenity::Context,
    &'a Event<'a>,
    FrameworkContext<'a>,
    &'a Data,
);

impl TriggerCooldown {
    const DURATION: std::time::Duration = std::time::Duration::from_secs(5);

    pub async fn on_cooldown(&self, user: serenity::UserId) -> bool {
        self.0
            .read()
            .await
            .get(&user)
            .is_some_and(|x| x.elapsed() < Self::DURATION)
    }

    pub async fn activate(&self, user: serenity::UserId) {
        self.0.write().await.insert(user, std::time::Instant::now());
    }

    pub async fn clean(&self) {
        self.0
            .write()
            .await
            .drain_filter(|_, x| x.elapsed() > Self::DURATION); // .for_each(|_| ());
    }
}

pub async fn get_alert_channel(
    guild: &serenity::Guild,
    reference: EventReference<'_>,
) -> Result<serenity::ChannelId, Error> {
    let prompt_channel: serenity::ChannelId;
    if let Some(channel) = guild.public_updates_channel_id.or(guild.system_channel_id) {
        prompt_channel = channel;
    } else if let Some(channel) = guild.default_channel(reference.2.bot_id).await {
        prompt_channel = channel.into();
    } else {
        return Err(FedBotError::new(format!(
            "cannot access any channels in guild '{}'",
            guild.name
        ))
        .into());
    }
    Ok(prompt_channel)
}

#[derive(FromQueryResult)]
struct ModLogData {
    mod_channel: i64,
}
#[instrument(skip_all, err)]

pub async fn mod_log(
    ctx: &serenity::Context,
    data: &Data,
    guild: serenity::GuildId,
    channel: Option<serenity::ChannelId>,
    msg: impl std::fmt::Display,
) -> Result<(), Error> {
    if let Some(x) = channel {
        x
    } else {
        let server_data: ModLogData = Servers::find_by_id(guild.as_u64().repack())
            .select_only()
            .column(servers::Column::Id)
            .column(servers::Column::ModChannel)
            .into_model()
            .one(&data.db)
            .await?
            .ok_or(FedBotError::new("Failed to find query"))?;
        serenity::ChannelId(server_data.mod_channel.repack())
    }
    .send_message(ctx, |f| {
        f.content(msg).allowed_mentions(|f| f.empty_users())
    })
    .await?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct FedBotError {
    msg: String,
}

impl error::Error for FedBotError {}

impl fmt::Display for FedBotError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "FedBotError: {}", self.msg)
    }
}

impl FedBotError {
    pub fn new<T: AsRef<str>>(msg: T) -> FedBotError {
        FedBotError {
            msg: msg.as_ref().to_owned(),
        }
    }
}

pub trait ContainBytes<T> {
    fn repack(&self) -> T;
}

impl ContainBytes<i64> for u64 {
    fn repack(&self) -> i64 {
        i64::from_ne_bytes(self.to_ne_bytes())
    }
}

impl ContainBytes<u64> for i64 {
    fn repack(&self) -> u64 {
        u64::from_ne_bytes(self.to_ne_bytes())
    }
}
