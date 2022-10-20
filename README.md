# FedBot: Discord Bot of Assorted Uses

### Usage
Required programs:
* [Python 3.8+](https://www.python.org/downloads)
* [Git](https://git-scm.com/download)

Clone repository:
```
git clone https://github.com/cyanoj/fedbot.git
```

Setup dependencies:
```
cd fedbot
pip install -r requirements.txt
```

[Generate a token](https://discord.com/developers/docs/getting-started#configuring-a-bot) for your Discord application.
Add the token to FedBot's settings (replace `$TOKEN` with your bot token, make sure to keep the quotes!):
```
echo DISCORD_FEDBOT_TOKEN="$TOKEN" > .env
```

Create a `profiles.toml` file (see [this section](#profiles-file-structure) for the file's structure).

Run FedBot:
```
python fedbot.py
```

### Profiles file structure

The `profiles.toml` file has a separate entry for each server profile. You should make one profile for every server you wish to use FedBot on. The server profile should be structured as follows:
```
[My-SuperAwesome_Server]
server_id = SERVERID
member_role = MEMBERROLEID
moderator_role = MODERATORROLEID
wait_channel = WAITCHANNELID
laws_channel = LAWSCHANNELID
```
* The server nickname at the brackets does not have to match the server name; it can be anything you like as long as it is a valid TOML key.
* The server ID should be the ID of the server the profile is for.
* For the member role, make a new role. Anyone without this role should be confined to only one channel, the wait channel.
* Anyone with the moderator role will have control over FedBot. You should only give this role to people you trust.
* The laws channel should be where your ~~laws~~ <ins>totally awesome server rules</ins> are posted so everyone knows what's acceptable and what's not.
* Every ID should be a TOML number. To get IDs from Discord, first ensure [Developer Mode](https://discord.com/developers/docs/game-sdk/store#application-test-mode) is turned on in your Discord settings than look in the context menu after right clicking on the server/role/user you are trying to get the ID for.

### Support
Questions? [Open an issue](https://github.com/cyanoj/fedbot/issues) on GitHub.

### Licensing
FedBot is licensed under the [Apache License, version 2.0](https://www.apache.org/licenses/LICENSE-2.0).
<br><br>
FedBot's logo ([avatar.png](avatar.png)) is also licensed under the [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) license. You are free to use this file and logo under either the Apache-2.0 or CC BY 4.0 licenses.