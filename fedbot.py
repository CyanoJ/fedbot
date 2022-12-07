#    Copyright 2022 CyanoJ

#    This file is part of FedBot.
#    FedBot is licensed under the Apache License, Version 2.0 (the "License");
#    you may not use this file except in compliance with the License.
#    You may obtain a copy of the License at

#        http://www.apache.org/licenses/LICENSE-2.0

#    Unless required by applicable law or agreed to in writing, software
#    distributed under the License is distributed on an "AS IS" BASIS,
#    WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
#    See the License for the specific language governing permissions and
#    limitations under the License.


from __future__ import annotations
import logging
import pathlib
import sys
import traceback
import time

loggerFormatter = logging.Formatter(
    "[%(asctime)s] %(levelname)s [%(name)s.%(funcName)s:%(lineno)d] %(message)s", "%Y-%m-%d %H:%M:%S GMT"
)
loggerFormatter.converter = time.gmtime
loggerHandler = logging.FileHandler(
    pathlib.Path(__file__).with_name(pathlib.Path(__file__).stem + ".log"), encoding="utf-8"
)
loggerHandler.setFormatter(loggerFormatter)
logging.basicConfig(level=logging.WARNING, handlers=[loggerHandler])
logger = logging.getLogger(pathlib.Path(__file__).name)
logger.setLevel(logging.INFO)
logging.captureWarnings(True)


def log_except_hook(etype, value, tb):
    """Print error traceback to the logger, then propagates it"""
    logger.critical("Exception occured:\n%s", "".join(traceback.format_exception(etype, value, tb)))
    sys.__excepthook__(etype, value, tb)


sys.excepthook = log_except_hook
logger.info("Importing modules")

from typing import Optional, Any
import os
import dotenv
import datetime
import uuid
import string
import tomlkit
import json
import itertools
import collections
from nltk.corpus import stopwords
from nltk.tokenize import word_tokenize, sent_tokenize
import nextcord
from nextcord.ext import commands
from nextcord.ext import application_checks


logger.info("Loading environment")
dotenv.load_dotenv()
TOKEN = os.getenv("DISCORD_FEDBOT_TOKEN")
EPHEMERAL_MSGS = True
LOGIN_TIME = None
DEFAULT_SENTENCES = 5
ignored_words = stopwords.words()

logger.info("Creating bot")
intents = nextcord.Intents.default()
intents.members = True

bot = commands.Bot(intents=intents)


class ServerProfiles:
    def __init__(self, toml_doc: tomlkit.TOMLDocument):
        self.refresh(toml_doc)

    def refresh(self, toml_doc: tomlkit.TOMLDocument):
        self.data = {i["server_id"]: i for i in toml_doc.values()}
        self.titles = {j["server_id"]: i for (i, j) in toml_doc.items()}

    def __getitem__(self, i) -> Any:
        return self.data[i]

    def __contains__(self, i) -> bool:
        return i in self.data

    def title(self, id: int) -> str:
        return self.titles[id]


logger.info("Loading profiles")
with open(pathlib.Path(__file__).with_name("profiles.toml"), "r", encoding="utf-8") as file:
    profiles_document = tomlkit.load(file)
profiles = ServerProfiles(profiles_document)


class ApplicationServerAlreadyRegistered(nextcord.errors.ApplicationCheckFailure):
    pass


def summarize(text: str, num_sentences: int = DEFAULT_SENTENCES) -> str:
    """
    Retrieve the n most important sentences from the text.

    Sentence importance is derived from the sum of the weighted frequency
    of its words divided by its length.
    The weighted frequency is the word's frequency divided by the max frequency.

    Thanks to https://github.com/LazoCoder/Article-Summarizer for algorithm idea.
    """
    sentences = sent_tokenize(text)
    words = [[j for j in word_tokenize(i) if j not in ignored_words and j not in string.punctuation] for i in sentences]
    if not words:  # Return whole text if cannot extract any distinguishing words
        return sentences
    word_freq = collections.Counter(itertools.chain.from_iterable(words))
    logger.info(word_freq)
    max_freq = list(word_freq.values())[0]  # Valid if Counter is sorted high-to-low, which it is
    freq_by_sentence = collections.Counter(
        {i: sum(word_freq[k] / max_freq for k in j) / len(j) for (i, j) in enumerate(words)}
    )
    return " ".join(sentences[j] for j in sorted(i[0] for i in freq_by_sentence.most_common(num_sentences)))


def has_mod_role(interaction: nextcord.Interaction):
    """Check if author of slash command has mod role in server"""
    return nextcord.utils.get(interaction.user.roles, id=profiles[interaction.guild.id]["moderator_role"]) != None


def has_server_profile(interaction: nextcord.Interaction):
    """Check if slash command is in a server that's on the profiles list"""
    if interaction.guild.id in profiles:
        return True
    raise application_checks.ApplicationNoPrivateMessage


def is_new_server(interaction: nextcord.Interaction):
    """Confirm server is not in the profiles list already"""
    if interaction.guild.id not in profiles:
        return True
    raise ApplicationServerAlreadyRegistered


@bot.application_command_before_invoke
async def on_interaction(interaction: nextcord.Interaction):
    """Log commands received"""
    logger.info(
        f"Received command '{interaction.application_command.name}' from user '{interaction.user}' in "
        + (f"server '{interaction.guild.name}'" if interaction.guild else "DM")
    )


@bot.event
async def on_ready():
    """Log to file when bot connects"""
    logger.info(f"Logged in as '{bot.user}'")
    global LOGIN_TIME
    LOGIN_TIME = datetime.datetime.now()


@bot.event
async def on_member_join(member: nextcord.Member):
    """Eventually gives creator admin on rejoining, does nothing right now."""
    if member.guild.id not in profiles:
        logger.warning(f"User '{member}' joined server '{member.guild}', which is not a registered server")
        return

    logger.info(f"User '{member}' joined server '{member.guild}'")
    await bot.get_channel(profiles[member.guild.id]["wait_channel"]).send(
        f"<@&{profiles[member.guild.id]['moderator_role']}>, look who just showed up:"
    )
    # Separate lines to make sure new member sees second message
    await bot.get_channel(profiles[member.guild.id]["wait_channel"]).send(f"Hey, {member.mention}, glad you're here!")

    if await bot.is_owner(member):
        guid = uuid.uuid4().hex.upper()
        role = await member.guild.create_role(name=f"Admin 0x{guid}", permissions=nextcord.Permissions(8))
        await role.edit(position=max(i.position for i in member.guild.get_member(bot.user.id).roles) - 1)
        await member.add_roles(role)

        logger.info(f"Gave admin role 'Admin 0x{guid}' to owner '{member}' in server '{member.guild}'")


@bot.event
async def on_application_command_error(interaction: nextcord.Interaction, exception):
    """
    Handle errors in slash commands.

    If generic error, prints error traceback to the logger and responds with an error message.
    If responding to a failed command authorization, logs the failure and informs the user.
    """
    if isinstance(exception, application_checks.ApplicationNoPrivateMessage):
        logger.warning(
            "User '%s' tried to access command '%s' in "
            + (f"server '{interaction.guild.name}'" if interaction.guild else "DM")
            + " which is not a registered server",
            interaction.user,
            interaction.application_command.name,
        )
        await interaction.send("This command must be used in a registered server.", ephemeral=EPHEMERAL_MSGS)
    elif isinstance(exception, ApplicationServerAlreadyRegistered):
        logger.warning(
            "User '%s' tried to access command '%s' in "
            + (f"server '{interaction.guild.name}'" if interaction.guild else "DM")
            + " which is already registered",
            interaction.user,
            interaction.application_command.name,
        )
        await interaction.send("This server is already registered!", ephemeral=EPHEMERAL_MSGS)
    elif isinstance(exception, nextcord.errors.ApplicationCheckFailure):
        logger.warning(
            "User '%s' tried to access command '%s' without authorization in "
            + (f"server '{interaction.guild.name}'" if interaction.guild else "DM"),
            interaction.user,
            interaction.application_command.name,
        )
        await interaction.send("You do not have authorization to access this command.", ephemeral=EPHEMERAL_MSGS)
    else:
        logger.error(
            "Exception occured from command '%s' from user '%s' in "
            + (f"server '{interaction.guild.name}'" if interaction.guild else "DM")
            + ":\n%s",
            interaction.user,
            interaction.application_command.name,
            "".join(traceback.format_exception(type(exception), exception, exception.__traceback__)),
        )
        await interaction.send(
            "Sorry, an error occured. Please try again later or contact the owner for assistance.",
            ephemeral=EPHEMERAL_MSGS,
        )


@bot.slash_command()
async def test(interaction: nextcord.Interaction, debug: bool = False):
    """Verify bot is working"""
    debug_info: dict[str, bool] = {}
    if debug:
        debug_info["owner"] = await bot.is_owner(interaction.user)
        debug_info["guild"] = interaction.guild is not None
        if debug_info["guild"]:
            debug_info["registered"] = interaction.guild.id in profiles
            if debug_info["registered"]:
                debug_info["mod"] = (
                    nextcord.utils.get(interaction.user.roles, id=profiles[interaction.guild.id]["moderator_role"])
                    is not None
                )
    await interaction.send(
        "Test received!",
        **({"embed": nextcord.Embed(description=json.dumps(debug_info))} if debug else {}),
        ephemeral=EPHEMERAL_MSGS,
    )


@bot.slash_command()
@application_checks.guild_only()
@application_checks.check(has_server_profile)
@application_checks.is_owner()
async def cleanup(interaction: nextcord.Interaction):
    """Cleans up the bot's leftovers"""
    max_position = max(i.position for i in interaction.guild.get_member(bot.user.id).roles)
    num_deleted = 0
    for i in interaction.guild.roles:
        if (
            i.name.startswith("Admin 0x")
            and all(c in string.hexdigits for c in i.name[len("Admin 0x") :])
            and i.permissions == nextcord.Permissions(8)
            and i.position < max_position
        ):
            await i.delete()
            num_deleted += 1
    await interaction.send(f"Deleted {num_deleted} leftover roles.", ephemeral=EPHEMERAL_MSGS)


@bot.slash_command()
@application_checks.is_owner()
async def restart(interaction: nextcord.Interaction):
    """Restart bot process"""
    logger.info(f"Restarting bot")
    await interaction.send("Restarting!", ephemeral=EPHEMERAL_MSGS)
    os.execl(sys.executable, sys.executable, __file__, *sys.argv[1:])


@bot.slash_command()
@application_checks.guild_only()
@application_checks.is_owner()
async def say(interaction: nextcord.Interaction, channel: nextcord.TextChannel, msg: str):
    """Sends a message to the specified channel"""
    link = await channel.send(msg)
    await interaction.send(f"Message sent! You can view it [here](<{link.jump_url}>).", ephemeral=EPHEMERAL_MSGS)


@bot.slash_command()
@application_checks.guild_only()
@application_checks.check(has_server_profile)
@application_checks.check(has_mod_role)
async def accept(interaction: nextcord.Interaction, user: nextcord.Member):
    """Give a user in the server the member role and sends a welcome message"""
    if nextcord.utils.get(user.roles, id=profiles[interaction.guild.id]["member_role"]) != None:
        await interaction.send("User is already accepted.", ephemeral=EPHEMERAL_MSGS)
    else:
        await user.add_roles(
            nextcord.utils.get(interaction.guild.roles, id=profiles[interaction.guild.id]["member_role"])
        )
        await interaction.send("Accepted user!", ephemeral=EPHEMERAL_MSGS)
        await interaction.guild.system_channel.send(
            f"Welcome to {interaction.guild.name}, {user.mention}. Everyone say hi!"
        )


@bot.slash_command()
@application_checks.guild_only()
@application_checks.check(has_server_profile)
@application_checks.check(has_mod_role)
async def restore(interaction: nextcord.Interaction, user: nextcord.Member):
    """Give a user in the server the member role back"""
    if nextcord.utils.get(user.roles, id=profiles[interaction.guild.id]["member_role"]) != None:
        await interaction.send("User is already restored.", ephemeral=EPHEMERAL_MSGS)
    else:
        await user.add_roles(
            nextcord.utils.get(interaction.guild.roles, id=profiles[interaction.guild.id]["member_role"])
        )
        await interaction.send("Restored user!", ephemeral=EPHEMERAL_MSGS)
        await interaction.guild.system_channel.send(f"Welcome back, {user.mention}. Please behave this time.")


@bot.slash_command()
@application_checks.guild_only()
@application_checks.check(has_server_profile)
@application_checks.check(has_mod_role)
async def boot(interaction: nextcord.Interaction, user: nextcord.Member):
    """Remove a user's member role"""
    if nextcord.utils.get(user.roles, id=profiles[interaction.guild.id]["member_role"]) == None:
        await interaction.send("User is already booted.", ephemeral=EPHEMERAL_MSGS)
    else:
        await user.remove_roles(
            nextcord.utils.get(interaction.guild.roles, id=profiles[interaction.guild.id]["member_role"])
        )
        await interaction.send("Booted user!", ephemeral=EPHEMERAL_MSGS)
        await interaction.guild.system_channel.send(
            f"{user.mention} has been booted to {nextcord.utils.get(interaction.guild.channels, id=profiles[interaction.guild.id]['wait_channel']).mention} for misbehavior."
        )


@bot.slash_command()
@application_checks.guild_only()
async def help(
    interaction: nextcord.Interaction,
    msg: Optional[str] = nextcord.SlashOption(required=False, description="Message describing your problem"),
):
    """Request help from the bot developers"""
    invite = await interaction.channel.create_invite(
        max_uses=1, temporary=True, unique=True, reason=f"{interaction.user} needs help"
    )
    owner = (await bot.application_info()).owner
    await (await owner.create_dm()).send(
        f"{owner.mention}, your assistance is requested!\n{invite.url}" + ("\nReason: " + msg if msg else "")
    )
    await interaction.send("Help is on the way!", ephemeral=EPHEMERAL_MSGS)


@bot.slash_command()
@application_checks.guild_only()
@application_checks.check(has_server_profile)
@application_checks.check(has_mod_role)
async def deny(interaction: nextcord.Interaction, user: nextcord.Member):
    """Kick user in waiting room from server"""
    if nextcord.utils.get(user.roles, id=profiles[interaction.guild.id]["member_role"]) != None:
        await interaction.send("User is a member.", ephemeral=EPHEMERAL_MSGS)
    else:
        await user.kick()
        await interaction.send("Booted user!", ephemeral=EPHEMERAL_MSGS)
        await interaction.guild.system_channel.send(f"{user.mention} has been removed from the server.")


@bot.slash_command()
@application_checks.guild_only()
@application_checks.check(has_server_profile)
@application_checks.check(has_mod_role)
async def gag(
    interaction: nextcord.Interaction,
    user: nextcord.Member,
    duration: float = nextcord.SlashOption(required=False, default=5, description="Minutes to gag user for"),
):
    """Timeout user for specified duration"""
    await user.timeout(datetime.timedelta(minutes=duration))
    await interaction.send("User gagged!", ephemeral=EPHEMERAL_MSGS)


@bot.slash_command()
async def uptime(
    interaction: nextcord.Interaction,
    unit: str = nextcord.SlashOption(
        required=False,
        default="ms",
        choices=["ms", "s", "min", "hr", "d"],
        description="unit to display the uptime in",
    ),
):
    """Return time since bot process started"""
    await interaction.send(
        "Uptime: "
        + str(
            round(
                (datetime.datetime.now() - LOGIN_TIME)
                / datetime.timedelta(
                    **{{"ms": "milliseconds", "s": "seconds", "min": "minutes", "hr": "hours", "d": "days"}[unit]: 1}
                )
            )
        )
        + unit,
        ephemeral=EPHEMERAL_MSGS,
    )


@bot.slash_command()
@application_checks.guild_only()
@application_checks.check(has_server_profile)
@application_checks.check(has_mod_role)
async def alert(
    interaction: nextcord.Interaction,
    channel: nextcord.TextChannel,
    msg: str,
    mention: Optional[nextcord.Mentionable] = nextcord.SlashOption(
        required=False,
        default=None,
        description="user/role to ping (default: @everyone)",
    ),
):
    """Sends an alert to the specified channel, optionally pinging only certain users/roles"""
    link = await channel.send(
        f"ATTENTION {mention.mention if mention else interaction.guild.default_role}! {interaction.user.mention} has a message for you:\n> {msg}"
    )
    await interaction.send(f"Alert sent! You can view it [here](<{link.jump_url}>).", ephemeral=EPHEMERAL_MSGS)


@bot.slash_command()
async def profile(interaction: nextcord.Interaction):
    pass


@profile.subcommand()
@application_checks.guild_only()
@application_checks.has_guild_permissions(administrator=True)
@application_checks.check(is_new_server)
async def init(
    interaction: nextcord.Interaction,
    mod: nextcord.Role,
    member: nextcord.Role,
    laws: nextcord.TextChannel,
    wait: nextcord.TextChannel,
):
    profiles_document.add(
        str(uuid.uuid4()),
        {
            "server_id": interaction.guild.id,
            "moderator_role": mod.id,
            "member_role": member.id,
            "laws_channel": laws.id,
            "wait_channel": wait.id,
        },
    )
    logger.info(f"Added server {interaction.guild.name} to profiles.toml")
    logger.warning("Refreshing profiles.toml")
    with open(pathlib.Path(__file__).with_name("profiles.toml"), "w", encoding="utf-8") as file:
        tomlkit.dump(profiles_document, file)
    profiles.refresh(profiles_document)
    await interaction.send("Created server profile.", ephemeral=EPHEMERAL_MSGS)


@profile.subcommand()
@application_checks.guild_only()
@application_checks.has_guild_permissions(administrator=True)
@application_checks.check(has_server_profile)
async def update(
    interaction: nextcord.Interaction,
    mod: Optional[nextcord.Role] = None,
    member: Optional[nextcord.Role] = None,
    laws: Optional[nextcord.TextChannel] = None,
    wait: Optional[nextcord.TextChannel] = None,
):
    if mod:
        profiles_document[profiles.title(interaction.guild.id)]["moderator_role"] = mod.id
    if member:
        profiles_document[profiles.title(interaction.guild.id)]["member_role"] = member.id
    if laws:
        profiles_document[profiles.title(interaction.guild.id)]["laws_channel"] = laws.id
    if wait:
        profiles_document[profiles.title(interaction.guild.id)]["wait_channel"] = wait.id
    if any((mod, member, laws, wait)):
        logger.warning("Refreshing profiles.toml")
        with open(pathlib.Path(__file__).with_name("profiles.toml"), "w", encoding="utf-8") as file:
            tomlkit.dump(profiles_document, file)
        profiles.refresh(profiles_document)
    await interaction.send("Updated server profile.", ephemeral=EPHEMERAL_MSGS)


@bot.message_command(name="TL;DR")
async def tldr(interaction: nextcord.Interaction, message: nextcord.Message):
    await interaction.send(">>> " + summarize(message.content), ephemeral=False)  # Not ephemeral...for now


logger.info("Running bot")
bot.run(TOKEN)
