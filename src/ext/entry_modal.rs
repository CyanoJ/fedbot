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

use std::{cmp::Ordering, sync::Arc};

use super::ContainBytes;
use crate::{
    check_admin,
    entities::{prelude::*, *},
};
use futures_lite::stream::StreamExt;
use itertools::Itertools;
use poise::serenity_prelude as serenity;
use poise::Modal;
use sea_orm::*;
use serde::{Deserialize, Serialize};
use serenity::Mentionable;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
struct ModalInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    max: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    min: Option<u64>,
    label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    placeholder: Option<String>,
    required: bool,
    style: serenity::InputTextStyle,
}

struct PartialModalInput {
    max: Option<u64>,
    min: Option<u64>,
    label: Option<String>,
    placeholder: Option<String>,
    required: bool,
    style: Option<serenity::InputTextStyle>,
}

impl Default for PartialModalInput {
    fn default() -> Self {
        Self {
            required: true,
            max: Option::default(),
            min: Option::default(),
            label: Option::default(),
            placeholder: Option::default(),
            style: Option::default(),
        }
    }
}

impl PartialModalInput {
    // type Complete = ModalInput;

    fn into_complete(self) -> Result<Result<ModalInput, PartialModalInput>, super::FedBotError> {
        if self.min.is_some_and(|x| self.max.is_some_and(|y| x > y)) {
            return Ok(Err(self));
        }
        Ok(Ok(ModalInput {
            max: self.max,
            min: self.min,
            placeholder: self.placeholder,
            label: self.label.ok_or(super::FedBotError::new("missing label"))?,
            required: self.required,
            style: self.style.ok_or(super::FedBotError::new("missing style"))?,
        }))
    }

    fn is_complete(&self) -> bool {
        self.label.is_some() && self.style.is_some()
    }

    #[allow(clippy::too_many_lines)]
    fn build_modal<'a>(
        &self,
        f: &'a mut serenity::CreateComponents,
        already_completed: &[ModalInput],
    ) -> &'a mut serenity::CreateComponents {
        f.create_action_row(|f| {
            f.create_select_menu(|f| {
                f.custom_id("style").placeholder("Input Type").options(|f| {
                    f.set_options(
                        vec!["Short", "Paragraph"]
                            .into_iter()
                            .map(|x| {
                                let mut option = serenity::CreateSelectMenuOption::new(
                                    x.to_string(),
                                    x.to_string(),
                                );
                                if (self.style.is_some_and(|y| {
                                    matches!(y, serenity::InputTextStyle::Paragraph)
                                }) && x == "Paragraph")
                                    || (self.style.is_some_and(|y| {
                                        matches!(y, serenity::InputTextStyle::Short)
                                    }) && x == "Short")
                                {
                                    option.default_selection(true);
                                }
                                option
                            })
                            .collect(),
                    )
                })
            })
        })
        .create_action_row(|f| {
            f.create_select_menu(|f| {
                f.custom_id("minLength")
                    .placeholder("Minimum Length")
                    .disabled(self.style.is_none())
                    .options(|f| {
                        f.set_options(
                            vec![5, 10, 25, 100, 250, 500, 1000, 1500, 2000, 2500, 3000, 3500]
                                .into_iter()
                                .map(|x| {
                                    let mut option = serenity::CreateSelectMenuOption::new(
                                        x.to_string(),
                                        x.to_string(),
                                    );
                                    if self.min.is_some_and(|y| y == x) {
                                        option.default_selection(true);
                                    }
                                    option
                                })
                                .collect(),
                        )
                    })
            })
        })
        .create_action_row(|f| {
            f.create_select_menu(|f| {
                f.custom_id("maxLength")
                    .placeholder("Maximum Length")
                    .disabled(self.style.is_none())
                    .options(|f| {
                        f.set_options(
                            // match &self.style {
                            //     Some(serenity::InputTextStyle::Short) => vec![5, 10, 25, 100, 250, 500, 1000, 1500, 2000, 2500, 3000, 3500],
                            //     // _ => vec![1500, 2000, 2500, 3000, 3500],
                            // }
                            vec![5, 10, 25, 100, 250, 500, 1000, 1500, 2000, 2500, 3000, 3500]
                                .into_iter()
                                .map(|x| {
                                    let mut option = serenity::CreateSelectMenuOption::new(
                                        x.to_string(),
                                        x.to_string(),
                                    );
                                    if self.max.is_some_and(|y| y == x) {
                                        option.default_selection(true);
                                    }
                                    option
                                })
                                .collect(),
                        )
                    })
            })
        })
        .create_action_row(|f| {
            f.create_button(|f| {
                f.custom_id("isRequired")
                    .label("Required")
                    .style(if self.required {
                        serenity::ButtonStyle::Success
                    } else {
                        serenity::ButtonStyle::Danger
                    })
            })
            .create_button(|f| {
                f.custom_id("moreTextOptions")
                    .label("Set Label & Placeholder")
                    .style(serenity::ButtonStyle::Primary)
            })
        })
        .create_action_row(|f| {
            f.create_button(|f| {
                f.custom_id("addToModal")
                    .label("Add Input to Modal")
                    .disabled(!self.is_complete() || already_completed.len() >= 5)
                    .style(serenity::ButtonStyle::Primary)
            })
            .create_button(|f| {
                f.custom_id("createModal")
                    .label("Create Modal")
                    .disabled(already_completed.is_empty())
                    .style(serenity::ButtonStyle::Secondary)
            })
        })
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct ModalStructure(Vec<ModalInput>);

struct EntryModal<'a>(&'a ModalStructure);

impl<'a> Modal for EntryModal<'a> {
    fn create(
        defaults: Option<Self>,
        custom_id: String,
    ) -> serenity::CreateInteractionResponse<'static> {
        let mut x = serenity::CreateInteractionResponse::default();
        x.kind(serenity::InteractionResponseType::Modal)
            .interaction_response_data(|f| {
                f.custom_id(custom_id).title("Entry Form");
                if let Some(data) = defaults {
                    f.components(|f| {
                        for i in &data.0 .0 {
                            f.create_action_row(move |f| {
                                f.create_input_text(|f| {
                                    i.max.map(|x| f.max_length(x));
                                    i.min.map(|x| f.min_length(x));
                                    i.placeholder.as_ref().map(|x| f.placeholder(x));

                                    f.required(i.required)
                                        .label(&i.label)
                                        .style(i.style)
                                        .custom_id(format!(
                                            "{}{}",
                                            Uuid::new_v4()
                                                .as_simple()
                                                .encode_upper(&mut Uuid::encode_buffer()),
                                            &i.label
                                        ))
                                })
                            });
                        }
                        f
                    });
                }
                f
            });
        x
    }

    fn parse(_data: serenity::ModalSubmitInteractionData) -> Result<Self, &'static str> {
        unreachable!()
    }
}

#[derive(Debug, Modal)]
#[name = "Add Label & Placeholder"]
struct ModalCreatorForm {
    #[name = "Label"]
    #[max_length = "45"]
    label: String,
    #[name = "Placeholder"]
    #[max_length = "100"]
    placeholder: Option<String>,
}

#[tracing::instrument(skip_all, err)]
#[poise::command(slash_command, guild_only)]
pub async fn set_entry_modal(ctx: super::Context<'_>) -> Result<(), super::Error> {
    let guild = ctx
        .guild()
        .ok_or(super::FedBotError::new("command not in guild"))?
        .id;

    check_admin!(ctx, guild);

    let sentinel: Option<i64> = Servers::find_by_id(guild.as_u64().repack())
        .select_only()
        .column(servers::Column::Id)
        .into_tuple()
        .one(&ctx.data().db)
        .await?;

    if sentinel.is_none() {
        let maybe_command_id = serenity::Command::get_global_application_commands(ctx)
            .await?
            .iter()
            .find_map(|x| {
                if &x.name == "profile" {
                    Some(x.id)
                } else {
                    None
                }
            });
        ctx.send(|f| {
            f.ephemeral(ctx.data().is_ephemeral).content(format!(
                "No server profile! Use {} to create a profile first.",
                if let Some(x) = maybe_command_id {
                    format!("</profile init:{x}>")
                } else {
                    "`/profile init`".to_string()
                }
            ))
        })
        .await?;
        return Ok(());
    }

    let mut current_input = PartialModalInput::default();
    let mut modal_inputs = vec![];

    let msg = ctx
        .send(|f| {
            f.ephemeral(ctx.data().is_ephemeral)
                .content(concat!("Use the buttons below to build new text inputs for your entry modal.\n",
                "Once you are satisfied with the input, click \"Add Input to Modal\" to add it.\n",
                "Inputs added will be previewed below. Once you are finished, click \"Create Modal\" to create your new entry modal.")
            )
                .components(|f| current_input.build_modal(f, &modal_inputs))
        })
        .await?;

    let mut collector = msg
        .message()
        .await?
        .await_component_interactions(ctx)
        .author_id(ctx.author().id)
        .build();

    let mut to_respond: Option<std::sync::Arc<serenity::MessageComponentInteraction>> = None;
    while let Some(x) = collector.next().await {
        match x.data.custom_id.as_str() {
            "moreTextOptions" => {
                /* Tweak of poise::Modal::execute to fix "Interaction has already been acknowledged" error,
                   caused by using the original message's context after a response has already been sent
                   https://docs.rs/poise/0.5.4/src/poise/modal.rs.html#53-91
                   Licensed under the MIT license
                   https://docs.rs/crate/poise/0.5.4/source/LICENSE
                */
                x.create_interaction_response(ctx, |f| {
                    *f = ModalCreatorForm::create(None, "modalForTextModals".to_string());
                    f
                })
                .await?;
                let mut modal_collector = serenity::ModalInteractionCollectorBuilder::new(ctx)
                    .filter(|x| x.data.custom_id == "modalForTextModals")
                    .author_id(ctx.author().id)
                    .timeout(std::time::Duration::from_secs(3600))
                    .build();

                if let Some(raw_response) = modal_collector.next().await {
                    raw_response
                        .create_interaction_response(ctx, |f| {
                            f.kind(serenity::InteractionResponseType::DeferredUpdateMessage)
                        })
                        .await?;
                    let form = ModalCreatorForm::parse(raw_response.data.clone())?;

                    current_input.label = Some(form.label);
                    current_input.placeholder = form.placeholder;

                    msg.edit(ctx, |f| {
                        f.components(|f| current_input.build_modal(f, &modal_inputs))
                    })
                    .await?;
                }
            }
            "addToModal" => match current_input.into_complete()? {
                Ok(complete) => {
                    let new_content =
                        format!("{}\n`{}`", msg.message().await?.content, complete.label);
                    modal_inputs.push(complete);
                    current_input = PartialModalInput::default();
                    msg.edit(ctx, |f| {
                        f.content(new_content)
                            .components(|f| current_input.build_modal(f, &modal_inputs))
                    })
                    .await?;
                    x.create_interaction_response(ctx, |f| {
                        f.kind(serenity::InteractionResponseType::DeferredUpdateMessage)
                    })
                    .await?;
                }
                Err(partial) => {
                    current_input = partial;
                    x.defer(ctx).await?;
                    x.create_followup_message(ctx, |f| {
                        f.content("Minimum length must be smaller than maximum length!")
                            .ephemeral(ctx.data().is_ephemeral)
                    })
                    .await?;
                }
            },
            "style" => {
                current_input.style = x
                    .data
                    .values
                    .get(0)
                    .map(|x| match x.as_str() {
                        "Short" => Ok(serenity::InputTextStyle::Short),
                        "Paragraph" => Ok(serenity::InputTextStyle::Paragraph),
                        _ => Err(super::FedBotError::new("unknown text input style")),
                    })
                    .transpose()?;
                msg.edit(ctx, |f| {
                    f.components(|f| current_input.build_modal(f, &modal_inputs))
                })
                .await?;
                x.create_interaction_response(ctx, |f| {
                    f.kind(serenity::InteractionResponseType::DeferredUpdateMessage)
                })
                .await?;
            }
            "minLength" => {
                current_input.min = x
                    .data
                    .values
                    .get(0)
                    .map(|x| x.as_str().parse())
                    .transpose()?;
                x.create_interaction_response(ctx, |f| {
                    f.kind(serenity::InteractionResponseType::DeferredUpdateMessage)
                })
                .await?;
            }
            "maxLength" => {
                current_input.max = x
                    .data
                    .values
                    .get(0)
                    .map(|x| x.as_str().parse())
                    .transpose()?;
                x.create_interaction_response(ctx, |f| {
                    f.kind(serenity::InteractionResponseType::DeferredUpdateMessage)
                })
                .await?;
            }
            "isRequired" => {
                current_input.required = !current_input.required;
                msg.edit(ctx, |f| {
                    f.components(|f| current_input.build_modal(f, &modal_inputs))
                })
                .await?;
                x.create_interaction_response(ctx, |f| {
                    f.kind(serenity::InteractionResponseType::DeferredUpdateMessage)
                })
                .await?;
            }
            "createModal" => {
                x.defer(ctx).await?;
                to_respond = Some(x);
                break;
            }
            _ => (),
        }
    }

    if let Some(to_respond) = to_respond {
        let mut model: servers::ActiveModel = sea_orm::ActiveModelTrait::default();
        model.id = ActiveValue::Unchanged(guild.as_u64().repack());
        model.entry_modal = ActiveValue::Set(Some(rmp_serde::to_vec_named(&modal_inputs)?));
        model.update(&ctx.data().db).await?;

        display_entry_modal(ctx.serenity_context(), ctx.data(), guild).await?;
        to_respond
            .create_followup_message(ctx, |f| {
                f.ephemeral(ctx.data().is_ephemeral)
                    .content("Created new entry modal.")
            })
            .await?;
    } else {
        return Err(super::FedBotError::new("strange error occured and loop broke early").into());
    }

    Ok(())
}

#[derive(FromQueryResult)]
struct DisplayEntryModalData {
    screening_channel: i64,
    entry_modal: Option<Vec<u8>>,
}

const MAX_BULK_DELETE: usize = 100;

#[tracing::instrument(skip_all, err)]
pub async fn display_entry_modal(
    ctx: &serenity::Context,
    data: &super::Data,
    guild: serenity::GuildId,
) -> Result<(), super::Error> {
    let server_data: DisplayEntryModalData = Servers::find_by_id(guild.as_u64().repack())
        .select_only()
        .column(servers::Column::Id)
        .column(servers::Column::ScreeningChannel)
        .column(servers::Column::EntryModal)
        .into_model()
        .one(&data.db)
        .await?
        .ok_or(super::FedBotError::new("Failed to find query"))?;

    let screening_channel = serenity::ChannelId(server_data.screening_channel.repack());
    let mut msg_generator = screening_channel
        .messages(ctx, |f| f)
        .await?
        .into_iter()
        .filter_map(|x| {
            if x.author.id == ctx.cache.current_user_field(|y| y.id) {
                Some(x.id)
            } else {
                None
            }
        })
        .array_chunks::<MAX_BULK_DELETE>();
    for i in msg_generator.by_ref() {
        screening_channel.delete_messages(ctx, i).await?;
    }
    if let Some(x) = msg_generator.into_remainder() {
        let remainder = x.collect::<Vec<_>>();
        match remainder.len().cmp(&1) {
            Ordering::Equal => {
                screening_channel.delete_message(ctx, &remainder[0]).await?;
            }
            Ordering::Greater => {
                screening_channel.delete_messages(ctx, remainder).await?;
            }
            Ordering::Less => (),
        }
    }

    if let Some(x) = server_data.entry_modal {
        let msg = screening_channel.send_message(ctx, |f|
        f.content("Welcome! Please fill out this form so our mods can learn a little bit more about you. Thank you for your cooperation!").components(|f| f.create_action_row(|f| f.create_button(|f| f.custom_id("completeForm").label("Complete Form"))))).await?;
        tokio::spawn(listen_for_forms(
            msg.await_component_interactions(ctx).build(),
            data.db.clone(),
            x,
            ctx.http.clone(),
            ctx.shard.clone(),
            guild,
        ));
    } else {
        screening_channel
            .say(ctx, "Welcome. Please wait. Mods will be here shortly.")
            .await?;
    }
    Ok(())
}

#[derive(FromQueryResult)]
struct FormSubmitData {
    mod_channel: i64,
    mod_role: i64,
}

const MAX_TOTAL_EMBED_LENGTH: usize = 6000;

#[tracing::instrument(skip_all, err)]
async fn listen_for_forms(
    mut button_stream: serenity::ComponentInteractionCollector,
    db: sea_orm::DatabaseConnection,
    raw_modal: Vec<u8>,
    http: Arc<serenity::Http>,
    shard: serenity::ShardMessenger,
    guild: serenity::GuildId,
) -> Result<(), super::Error> {
    let modal_data: ModalStructure = rmp_serde::from_slice(&raw_modal)?;

    while let Some(evt) = button_stream.next().await {
        /* Tweak of poise::Modal::execute to run a modal without a Context
           https://docs.rs/poise/0.5.4/src/poise/modal.rs.html#53-91
           Licensed under the MIT license
           https://docs.rs/crate/poise/0.5.4/source/LICENSE
        */
        evt.create_interaction_response(&http, |f| {
            *f = EntryModal::create(Some(EntryModal(&modal_data)), "entryModal".to_string());
            f
        })
        .await?;
        let modal_collector = serenity::ModalInteractionCollectorBuilder::new(&shard)
            .filter(|x| x.data.custom_id == "entryModal")
            .author_id(evt.user.id)
            .timeout(std::time::Duration::from_secs(3600))
            .build();

        tokio::spawn(wait_for_modal(
            modal_collector,
            db.clone(),
            http.clone(),
            guild,
        ));
    }
    Ok(())
}

#[tracing::instrument(skip_all, err)]
async fn wait_for_modal(
    mut modal_collector: serenity::ModalInteractionCollector,
    db: sea_orm::DatabaseConnection,
    http: Arc<serenity::Http>,
    guild: serenity::GuildId,
) -> Result<(), super::Error> {
    if let Some(raw_response) = modal_collector.next().await {
        raw_response
            .create_interaction_response(&http, |f| {
                f.kind(serenity::InteractionResponseType::DeferredUpdateMessage)
            })
            .await?;

        let server_data: FormSubmitData = Servers::find_by_id(guild.as_u64().repack())
            .select_only()
            .column(servers::Column::Id)
            .column(servers::Column::ModChannel)
            .column(servers::Column::ModRole)
            .into_model()
            .one(&db)
            .await?
            .ok_or(super::FedBotError::new("Failed to find query"))?;

        let (mod_channel, mod_role) = (
            serenity::ChannelId(server_data.mod_channel.repack()),
            serenity::RoleId(server_data.mod_role.repack()),
        );

        let mut content = format!(
            "{}, user {} has submitted an entry form:",
            mod_role.mention(),
            raw_response.user.mention(),
        );
        let mut msg_embeds = vec![];
        let mut embeds_length: usize = 0;

        for (label, value) in raw_response
            .data
            .components
            .iter()
            .map(|x| {
                x.components
                    .iter()
                    .filter_map(|x| match x {
                        serenity::ActionRowComponent::InputText(y) => {
                            if let Some(label) = y.custom_id.get(uuid::fmt::Simple::LENGTH..) {
                                return Some((label, y.value.as_str()));
                            }
                            None
                        }
                        _ => None,
                    })
                    .collect::<Vec<(&str, &str)>>()
            })
            .concat()
        {
            let this_embed_length = raw_response.user.tag().len()
                + raw_response.user.face().len()
                + label.len()
                + value.len();

            if embeds_length + this_embed_length > MAX_TOTAL_EMBED_LENGTH {
                mod_channel
                    .send_message(&http, |f| f.content(content).add_embeds(msg_embeds))
                    .await?;
                content = String::new();
                msg_embeds = vec![];
                embeds_length = 0;
            }

            embeds_length += this_embed_length;
            let mut embed = serenity::CreateEmbed::default();
            embed.author(|f| {
                f.name(raw_response.user.tag())
                    .icon_url(raw_response.user.face())
                    .url(format!(
                        "https://discordapp.com/users/{}",
                        raw_response.user.id
                    ))
            });
            embed.title(label);
            embed.description(value);
            msg_embeds.push(embed);
        }
        if !msg_embeds.is_empty() {
            mod_channel
                .send_message(&http, |f| f.content(content).add_embeds(msg_embeds))
                .await?;
        }
    }
    Ok(())
}
