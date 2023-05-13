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

use super::{ApplicationContext, ContainBytes, Context, Error};
use crate::{
    check_mod_role,
    entities::{prelude::*, *},
};
use base64::{engine::general_purpose, Engine as _};
use chrono::{
    offset::Utc, DateTime, Datelike, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, Offset,
    TimeZone, Timelike,
};
use chrono_tz::TZ_VARIANTS;
use itertools::Itertools;
use poise::serenity_prelude as serenity;
use poise::Modal;
use rand::Rng;
use sea_orm::*;
use serenity::model::application::oauth::Scope;
use serenity::Mentionable;
use std::{cmp::Ordering, default::Default, fmt::Display};
use tracing::instrument;

#[derive(Debug, Clone, Copy)]
enum SweeperSquare {
    Mine,
    Clear(u8),
}

impl Default for SweeperSquare {
    fn default() -> Self {
        Self::Clear(0)
    }
}

impl Display for SweeperSquare {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "||{}||",
            match self {
                Self::Mine => "\u{1F4A5}".to_owned(),
                Self::Clear(x) => format!("{x}\u{fe0f}\u{20e3}"),
            }
            .as_str()
        ))
    }
}

#[derive(Debug)]
struct MineSweeper<const SIZE: usize>([[SweeperSquare; SIZE]; SIZE]);

impl<const SIZE: usize> Display for MineSweeper<SIZE> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for i in self.0 {
            f.write_fmt(format_args!("{}\n", i.iter().format(" ")))?;
        }
        Ok(())
    }
}

impl<const SIZE: usize> MineSweeper<SIZE> {
    fn _get_coords(selected: usize) -> (usize, usize) {
        let col = selected % SIZE;
        let row = ((SIZE - col + selected) / SIZE) - 1;
        (row, col)
    }

    fn new(mines: usize) -> Option<Self> {
        let squares = SIZE * SIZE;
        if mines > squares {
            return None;
        }

        let mut rng = rand::thread_rng();
        let mut sweeper = Self([[SweeperSquare::default(); SIZE]; SIZE]);
        for _ in 0..mines {
            let mut selected = rng.gen_range(0..squares);
            let (mut row, mut col) = Self::_get_coords(selected);

            while matches!(sweeper.0[row][col], SweeperSquare::Mine) {
                selected = (selected + 1) % squares;
                (row, col) = Self::_get_coords(selected);
            }

            sweeper.0[row][col] = SweeperSquare::Mine;

            for i in [
                if col > 0 { Some((col - 1, row)) } else { None },
                if col > 0 {
                    Some((col - 1, row + 1))
                } else {
                    None
                },
                Some((col, row + 1)),
                Some((col + 1, row + 1)),
                Some((col + 1, row)),
                if row > 0 {
                    Some((col + 1, row - 1))
                } else {
                    None
                },
                if row > 0 { Some((col, row - 1)) } else { None },
                if row > 0 && col > 0 {
                    Some((col - 1, row - 1))
                } else {
                    None
                },
            ]
            .into_iter()
            .flatten()
            {
                if (i.0 < SIZE) && (i.1 < SIZE) {
                    if let SweeperSquare::Clear(x) = &mut sweeper.0[i.1][i.0] {
                        *x += 1;
                    }
                }
            }
        }
        Some(sweeper)
    }
}

#[derive(Copy, Clone, Debug, poise::ChoiceParameter)]
#[repr(usize)]
pub enum MineSweeperSize {
    #[name = "Small"]
    Small = 4,
    #[name = "Medium"]
    Medium = 6,
    #[name = "Large"]
    Large = 9,
}

impl MineSweeperSize {
    const fn val(self) -> usize {
        self as usize
    }
}

#[derive(FromQueryResult)]
struct MoveMessageServerData {
    mod_role: i64,
}

#[derive(Modal)]
#[name = "Move to channel"]
struct MoveMessageModal {
    #[name = "Channel"]
    // #[placeholder = "#"]
    channel: String,
}

/// Play a fun minesweeper game
#[instrument(skip_all, err)]
#[poise::command(slash_command)]
pub async fn minesweeper(
    ctx: Context<'_>,
    size: MineSweeperSize,
    mines: usize,
) -> Result<(), Error> {
    if let Some(text) = match size {
        MineSweeperSize::Small => {
            MineSweeper::<{ MineSweeperSize::Small.val() }>::new(mines).map(|x| x.to_string())
        }
        MineSweeperSize::Medium => {
            MineSweeper::<{ MineSweeperSize::Medium.val() }>::new(mines).map(|x| x.to_string())
        }
        MineSweeperSize::Large => {
            MineSweeper::<{ MineSweeperSize::Large.val() }>::new(mines).map(|x| x.to_string())
        }
    } {
        ctx.send(|f| f.content(text)).await?;
    } else {
        ctx.send(|f| {
            f.ephemeral(ctx.data().is_ephemeral)
                .content("Too many mines!")
        })
        .await?;
    }
    Ok(())
}

const MAX_BULK_DELETE: usize = 100;

/// Purge all messages up to and including this one
#[instrument(skip_all, err)]
#[poise::command(guild_only, context_menu_command = "Purge To")]
pub async fn purgeto(ctx: Context<'_>, msg: serenity::Message) -> Result<(), Error> {
    let guild = ctx
        .guild_id()
        .ok_or(super::FedBotError::new("command must be used in guild"))?;

    let server_data: MoveMessageServerData = Servers::find_by_id(guild.as_u64().repack())
        .select_only()
        .column(servers::Column::Id)
        .column(servers::Column::ModRole)
        .into_model()
        .one(&ctx.data().db)
        .await?
        .ok_or(super::FedBotError::new("Failed to find query"))?;
    let (mod_role,) = (serenity::RoleId(server_data.mod_role.repack()),);

    check_mod_role!(ctx, guild, mod_role);

    let mut msg_generator = msg
        .channel_id
        .messages(ctx, |f| f.after(msg.id))
        .await?
        .into_iter()
        .map(|x| x.id)
        .array_chunks::<MAX_BULK_DELETE>();

    for i in msg_generator.by_ref() {
        msg.channel_id.delete_messages(ctx, i).await?;
    }
    if let Some(x) = msg_generator.into_remainder() {
        let remainder = x.collect::<Vec<_>>();
        match remainder.len().cmp(&1) {
            Ordering::Equal => {
                msg.channel_id.delete_message(ctx, &remainder[0]).await?;
            }
            Ordering::Greater => {
                msg.channel_id.delete_messages(ctx, remainder).await?;
            }
            Ordering::Less => (),
        }
    }

    msg.channel_id.delete_message(ctx, msg.id).await?; // Up to *and including*

    ctx.send(|f| {
        f.content("Purged messages.")
            .ephemeral(ctx.data().is_ephemeral)
    })
    .await?;
    Ok(())
}

#[allow(clippy::unused_async)]
pub async fn tz_autocomplete<'a>(
    _ctx: super::Context<'a>,
    partial: &'a str,
) -> impl Iterator<Item = poise::AutocompleteChoice<i32>> + 'a {
    let partial_matcher = partial.to_lowercase();
    let now = Utc::now().naive_utc();
    let mut all_tzs = TZ_VARIANTS
        .iter()
        .map(|x| poise::AutocompleteChoice {
            name: x.name().to_owned().replace('_', " "),
            value: x.offset_from_utc_datetime(&now).fix().local_minus_utc(),
        })
        .filter_map(|x| {
            let lower_name = x.name.to_lowercase();
            if lower_name.contains(&partial_matcher) {
                Some((x, lower_name))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    if !partial_matcher.is_empty() {
        all_tzs.sort_by_key(|x| {
            if x.1 == partial_matcher {
                0
            } else {
                x.1.find(&partial_matcher).unwrap_or(usize::MAX)
            }
        });
    }
    all_tzs.into_iter().map(|x| x.0).take(25)
}

/// Generate a Discord timestamp object
#[tracing::instrument(skip_all, err)]
#[poise::command(slash_command)]
#[allow(clippy::too_many_arguments)]
pub async fn timestamp(
    ctx: super::Context<'_>,
    #[autocomplete = "tz_autocomplete"] tz: i32,
    hour: u32,
    minute: u32,
    second: Option<u32>,
    year: Option<i32>,
    month: Option<u32>,
    day: Option<u32>,
) -> Result<(), super::Error> {
    let offset = FixedOffset::east_opt(tz).ok_or(super::FedBotError::new("unknown tz offset"))?;
    let now = Utc::now().with_timezone(&offset);
    let instant = NaiveDateTime::new(
        NaiveDate::from_ymd_opt(
            year.unwrap_or(now.year()),
            month.unwrap_or(now.month()),
            day.unwrap_or(now.day()),
        )
        .ok_or(super::FedBotError::new("unknown y/m/d"))?,
        NaiveTime::from_hms_opt(hour, minute, second.unwrap_or(now.second()))
            .ok_or(super::FedBotError::new("unknown h/m/s"))?,
    );
    let timestamp = DateTime::<FixedOffset>::from_local(instant, offset).timestamp();

    let mut format_code = None;
    if year.is_none() && month.is_none() && day.is_none() {
        if second.is_none() {
            format_code = Some("t");
        } else {
            format_code = Some("T");
        }
    }

    let code = format!(
        "<t:{}{}>",
        timestamp,
        format_code.map_or(String::new(), |x| format!(":{x}"))
    );
    ctx.send(|f| {
        f.content(format!("`{}` ({})", &code, &code))
            .ephemeral(ctx.data().is_ephemeral)
    })
    .await?;
    Ok(())
}

/// Verify bot is working
#[instrument(skip_all, err)]
#[poise::command(slash_command)]
pub async fn test(ctx: Context<'_>, debug: Option<bool>) -> Result<(), Error> {
    ctx.send(|f| {
        f.content("Test received!")
            .ephemeral(ctx.data().is_ephemeral);
        if debug.is_some_and(|val| val) {
            f.embed(|f| f.description("hi"));
        }
        f
    })
    .await?;
    Ok(())
}

/// Get invite link
#[instrument(skip_all, err)]
#[poise::command(slash_command)]
pub async fn invite(ctx: Context<'_>) -> Result<(), Error> {
    let invite_url = ctx
        .serenity_context()
        .cache
        .current_user()
        .invite_url_with_oauth2_scopes(
            ctx,
            serenity::Permissions::ADMINISTRATOR,
            &[Scope::Bot, Scope::ApplicationsCommands],
        )
        .await?;
    ctx.send(|f| f.content(invite_url).ephemeral(ctx.data().is_ephemeral))
        .await?;
    Ok(())
}

/// Create a poll
#[instrument(skip_all, err)]
#[poise::command(slash_command)]
pub async fn poll(
    ctx: Context<'_>,
    question: String,
    #[description = "Poll options, separated by semicolons"] options: String,
) -> Result<(), Error> {
    let options_vec = options.split(';').map(str::trim).collect::<Vec<&str>>();
    let options_length = options_vec.len();
    if options_length < 2 {
        ctx.send(|f| {
            f.content("You must specify at least two options, separated by semicolons.")
                .ephemeral(ctx.data().is_ephemeral)
        })
        .await?;
        return Ok(());
    }
    if options_length > 26 {
        ctx.send(|f| {
            f.content("Too many options!")
                .ephemeral(ctx.data().is_ephemeral)
        })
        .await?;
        return Ok(());
    }
    let mut formatted_options = vec![];
    for (val, index) in options_vec.iter().zip(0..u32::MAX) {
        formatted_options.push(format!(
            "{}: {}",
            char::from_u32('\u{1f1e6}' as u32 + index)
                .ok_or(super::FedBotError::new("Unicode decode error"))?,
            val
        ));
    }
    let msg = ctx
        .send(|f| {
            f.embed(|f| {
                f.title(question)
                    .description(formatted_options.into_iter().format("\n"))
            })
        })
        .await?
        .into_message()
        .await?;
    for i in 0..options_length.try_into()? {
        msg.react(
            ctx,
            char::from_u32('\u{1f1e6}' as u32 + i)
                .ok_or(super::FedBotError::new("Unicode decode error"))?,
        )
        .await?;
    }
    Ok(())
}

#[derive(Debug, Modal)]
#[name = "Set Emoji Name"]
struct PirateEmojiName {
    #[name = "Emoji Name"]
    #[placeholder = "Leave blank to use the emoji's original name"]
    #[max_length = "32"]
    #[min_length = "2"]
    name: Option<String>,
}

#[instrument(skip_all, err)]
#[poise::command(context_menu_command = "Pirate Emoji", guild_only)]
pub async fn pirate_emoji(ctx: Context<'_>, msg: serenity::Message) -> Result<(), Error> {
    let modal_ctx: ApplicationContext;
    if let Context::Application(inner_ctx) = ctx {
        modal_ctx = inner_ctx;
    } else {
        return Err(super::FedBotError::new("command must be used in application context").into());
    }

    let guild = ctx
        .guild_id()
        .ok_or(super::FedBotError::new("command must be used in guild"))?;

    let server_data: MoveMessageServerData = Servers::find_by_id(guild.as_u64().repack())
        .select_only()
        .column(servers::Column::Id)
        .column(servers::Column::ModRole)
        .into_model()
        .one(&ctx.data().db)
        .await?
        .ok_or(super::FedBotError::new("Failed to find query"))?;
    let (mod_role,) = (serenity::RoleId(server_data.mod_role.repack()),);

    check_mod_role!(ctx, guild, mod_role);

    let mut emojis = super::EMOJI.captures_iter(&msg.content);

    let Some(to_pirate) = emojis.next() else {
            ctx.send(|f| {
                f.content("No emojis in message!")
                    .ephemeral(ctx.data().is_ephemeral)
            })
            .await?;
            return Ok(());
        };

    let mut pirate_name = to_pirate
        .get(2)
        .ok_or(super::FedBotError::new("regex malfunction on name"))?
        .as_str();
    let pirate_name_guard: String;
    let pirate_id = to_pirate
        .get(3)
        .ok_or(super::FedBotError::new("regex malfunction on id"))?
        .as_str();

    if emojis.next().is_some() {
        ctx.send(|f| {
            f.content("More than one emoji in message!")
                .ephemeral(ctx.data().is_ephemeral)
        })
        .await?;
        return Ok(());
    }

    let emoji_encoding = if to_pirate
        .get(1)
        .ok_or(super::FedBotError::new(
            "regex malfunction on animated sentinel",
        ))?
        .as_str()
        .is_empty()
    {
        "png"
    } else {
        "gif"
    };

    if let Some(x) = PirateEmojiName::execute_with_defaults(
        modal_ctx,
        PirateEmojiName {
            name: Some(pirate_name.to_owned()),
        },
    )
    .await?
    {
        if let Some(y) = x.name {
            pirate_name_guard = y;
            pirate_name = &pirate_name_guard;
        }
    }

    let new_emoji = guild
        .create_emoji(
            ctx,
            pirate_name,
            &format!(
                "data:image/{};base64,{}",
                emoji_encoding,
                general_purpose::STANDARD.encode(
                    ctx.data()
                        .reqwest
                        .get(format!(
                            "https://cdn.discordapp.com/emojis/{pirate_id}.{emoji_encoding}",
                        ))
                        .send()
                        .await?
                        .bytes()
                        .await?
                )
            ),
        )
        .await?;

    ctx.send(|f| {
        f.content(format!("\u{1f3f4}\u{200d}\u{2620}\u{fe0f} {new_emoji}"))
            .ephemeral(ctx.data().is_ephemeral)
    })
    .await?;
    Ok(())
}

#[instrument(skip_all, err)]
#[poise::command(context_menu_command = "Move", guild_only)]
pub async fn move_(ctx: Context<'_>, msg: serenity::Message) -> Result<(), Error> {
    let modal_ctx: ApplicationContext;
    if let Context::Application(inner_ctx) = ctx {
        modal_ctx = inner_ctx;
    } else {
        return Err(super::FedBotError::new("command must be used in application context").into());
    }

    let guild = ctx
        .guild_id()
        .ok_or(super::FedBotError::new("command must be used in guild"))?;

    let server_data: MoveMessageServerData = Servers::find_by_id(guild.as_u64().repack())
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

    let data = MoveMessageModal::execute(modal_ctx)
        .await?
        .ok_or(super::FedBotError::new("no response"))?;

    let channels = guild.channels(ctx).await?;
    let channel = channels
        .values()
        .find(|x| x.name == data.channel)
        .ok_or(super::FedBotError::new("could not find channel"))?;

    crate::defer!(ctx);

    let webhook = match msg.author.avatar_url() {
        Some(avatar) => {
            channel
                .create_webhook_with_avatar(ctx, &msg.author.name, avatar.as_str())
                .await?
        }
        None => channel.create_webhook(ctx, &msg.author.name).await?,
    };

    webhook
        .execute(ctx, true, |f| {
            f.content(&msg.content).files(
                msg.attachments
                    .iter()
                    .map(|x| x.url.as_str())
                    .collect::<Vec<&str>>(),
            )
        })
        .await?;

    webhook.delete(ctx).await?;
    msg.reply(
        ctx,
        format!(
            "{}, your message has been moved to {}",
            msg.author.mention(),
            channel.mention()
        ),
    )
    .await?;
    msg.channel_id.delete_message(ctx, msg.id).await?;

    ctx.send(|f| {
        f.ephemeral(ctx.data().is_ephemeral)
            .content(format!("Moved message to {}", channel.mention()))
    })
    .await?;

    Ok(())
}
