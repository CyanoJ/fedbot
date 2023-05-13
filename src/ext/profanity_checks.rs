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

use dunce::canonicalize;
use lazy_static::lazy_static;
use poise::serenity_prelude as serenity;
use rustrict::{Censor, Type};
use serenity::Mentionable;
use std::path::Path;
use tracing::{info, instrument};

lazy_static! {
    static ref CENSOR_BANNED: rustrict::Banned = {
        let path = canonicalize(Path::new(&std::env::current_exe().unwrap()))
            .unwrap()
            .with_file_name("banned_chars.txt");
        let mut banned = rustrict::Banned::new();
        if let Some(x) = match std::fs::read_to_string(path) {
            Ok(x) => Ok(Some(x)),
            Err(e) => match e.kind() {
                std::io::ErrorKind::NotFound => Ok(None),
                other => Err(other),
            },
        }
        .unwrap()
        {
            for i in x.lines().filter_map(|x| x.chars().next()) {
                banned.insert(i);
            }
        }
        banned
    };
    static ref CENSOR_REPLACEMENTS: rustrict::Replacements = {
        let path = canonicalize(Path::new(&std::env::current_exe().unwrap()))
            .unwrap()
            .with_file_name("replace_chars.txt");
        let mut replacements = rustrict::Replacements::new();
        if let Some(x) = match std::fs::read_to_string(path) {
            Ok(x) => Ok(Some(x)),
            Err(e) => match e.kind() {
                std::io::ErrorKind::NotFound => Ok(None),
                other => Err(other),
            },
        }
        .unwrap()
        {
            for (src, dest) in x.lines().filter_map(|x| {
                let mut line = x.chars();
                line.next().and_then(|y| line.next().map(|z| (y, z)))
            }) {
                replacements.insert(src, dest);
            }
        }
        replacements
    };
    static ref CENSOR_TRIE: rustrict::Trie = {
        let allow_path = canonicalize(Path::new(&std::env::current_exe().unwrap()))
            .unwrap()
            .with_file_name("allowlist.txt");
        let block_path = canonicalize(Path::new(&std::env::current_exe().unwrap()))
            .unwrap()
            .with_file_name("blocklist.txt");
        let mut trie = rustrict::Trie::new();
        if let Some(x) = match std::fs::read_to_string(allow_path) {
            Ok(x) => Ok(Some(x)),
            Err(e) => match e.kind() {
                std::io::ErrorKind::NotFound => Ok(None),
                other => Err(other),
            },
        }
        .unwrap()
        {
            for i in x.lines() {
                trie.set(i.to_lowercase().as_str(), Type::SAFE);
            }
        }
        if let Some(x) = match std::fs::read_to_string(block_path) {
            Ok(x) => Ok(Some(x)),
            Err(e) => match e.kind() {
                std::io::ErrorKind::NotFound => Ok(None),
                other => Err(other),
            },
        }
        .unwrap()
        {
            for i in x.lines() {
                trie.set(i.to_lowercase().as_str(), Type::PROFANE & Type::SEVERE);
            }
        }
        trie
    };
}

pub fn init_statics() {
    lazy_static::initialize(&CENSOR_BANNED);
    lazy_static::initialize(&CENSOR_REPLACEMENTS);
    lazy_static::initialize(&CENSOR_TRIE);
}

pub trait Censorable {
    fn check_profanity(&self) -> Option<&str>;
}

impl<T: Censorable> Censorable for Option<T> {
    #[inline]
    fn check_profanity(&self) -> Option<&str> {
        self.as_ref().and_then(Censorable::check_profanity)
    }
}

impl<T: Censorable> Censorable for Vec<T> {
    #[inline]
    fn check_profanity(&self) -> Option<&str> {
        self.iter().find_map(Censorable::check_profanity)
    }
}

macro_rules! censor_tuple_enum {
    ($x:ty, $($y:ident),+) => {
        impl Censorable for $x {
            #[inline]
            fn check_profanity(&self) -> Option<&str> {
                match self {
                    $(Self::$y(val) => val.check_profanity(),)+
                    _ => None
                }
            }
        }
    };
}

macro_rules! censor_impl {
    ($x:ty) => {
        impl Censorable for $x {
            fn check_profanity(&self) -> Option<&str> {
                let scan_types = Censor::new(self.to_lowercase().chars().filter_map(|x|
                    // Convert dashes and newlines to spaces to trigger false positive detection
                    if x == '\n' || x == '-' {Some(' ')}
                    // Remove asterisks to stop self-censor detection for markdown bolding
                    else if x == '*' {None}
                    // Replace regional_indicator characters with their ASCII equivalents
                    else if ('\u{1f1e6}'..='\u{1f1ff}').contains(&x) {Some((x as u8 - ('\u{1f1e6}' as u8 - 'a' as u8)) as char)}
                    // Keep other characters unchanged
                    else {Some(x)})
                )
                .with_trie(&CENSOR_TRIE)
                .with_replacements(&CENSOR_REPLACEMENTS)
                .with_ignore_false_positives(false)
                .analyze();
                if (scan_types.is(Type::PROFANE) & !scan_types.is(Type::EVASIVE))
                | (scan_types.is(Type::SEXUAL) & !scan_types.is(Type::EVASIVE))
                | scan_types.is(Type::PROFANE & Type::MODERATE_OR_HIGHER & Type::EVASIVE)
                | scan_types.is(Type::PROFANE & Type::MODERATE_OR_HIGHER & Type::EVASIVE) {
                    Some(self)
                } else {
                    None
                }
            }
        }
    };
    ($x:ty, $y:ident $(, $z:ident)*) => {
        impl Censorable for $x {
            #[inline]
            fn check_profanity(&self) -> Option<&str> {
                self.$y.check_profanity()
                $( .or_else(|| self.$z.check_profanity()) )*
            }
        }
    };
}

censor_impl! {String}
censor_impl! {&str}

censor_impl! {serenity::MessageUpdateEvent, content, attachments, embeds}
censor_impl! {&serenity::MessageUpdateEvent, content, attachments, embeds}
censor_impl! {serenity::Message, content, attachments, embeds, components}
censor_impl! {&serenity::Message, content, attachments, embeds, components}
censor_impl! {serenity::Attachment, url, filename}

censor_impl! {serenity::ActionRow, components}
censor_tuple_enum! {serenity::ActionRowComponent, Button, SelectMenu, InputText}
censor_impl! {serenity::Button, label, url}
censor_impl! {serenity::SelectMenu, placeholder, options, values}
censor_impl! {serenity::SelectMenuOption, label, value}
censor_impl! {serenity::InputText, value}

censor_impl! {serenity::Embed, author, description, fields, footer, image, thumbnail, title, url, video}
censor_impl! {serenity::EmbedThumbnail, url}
censor_impl! {serenity::EmbedVideo, url}
censor_impl! {serenity::EmbedImage, url}
censor_impl! {serenity::EmbedFooter, text, icon_url}
censor_impl! {serenity::EmbedAuthor, name, url, icon_url}
censor_impl! {serenity::EmbedField, name, value}

#[instrument(skip_all, err)]
pub async fn filter_message<T: Censorable>(
    filter: T,
    channel: serenity::ChannelId,
    id: serenity::MessageId,
    author: &serenity::User,
    reference: super::EventReference<'_>,
) -> Result<bool, super::Error> {
    if let Some(objectionable) = filter.check_profanity() {
        channel.delete_message(&reference.0, id).await?;
        channel
            .send_message(&reference.0, |f| {
                f.content(format!(
                    "Deleted message from {} (reason: profanity)",
                    author.mention()
                ))
            })
            .await?;
        info!(
            "Deleted profane message from '{}#{}' (content: '{}')",
            author.name, author.discriminator, objectionable
        );
        return Ok(true);
    }
    Ok(false)
}
