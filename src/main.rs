use std::env;
use std::fmt::Display;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use autocomplete::AutoCompleteEngine;
use dotenvy::dotenv;
use inflector::Inflector;
use log::{debug, error, info};
use reqwest::{self, IntoUrl, Url};
use serde_derive::{Deserialize, Serialize};
use urlencoding::encode;

use poise::samples::register_application_commands_buttons;
use poise::{CreateReply, PrefixFrameworkOptions};
use serenity::builder::CreateEmbed;
use serenity::client::ClientBuilder;
use serenity::prelude::*;

mod autocomplete;

struct State {
    pal_names: Vec<String>,
    ac_eng: Arc<AutoCompleteEngine>,
    pal_api_url: Url,
}

impl State {
    pub async fn new(pal_api_url: &str) -> Result<Self> {
        let pal_api_url = Url::parse(pal_api_url).expect("Invalid URL for the Palworld API");
        let mut pal_names = get_pal_names(&pal_api_url).await?;
        pal_names.sort();

        Ok(Self {
            ac_eng: Arc::new(AutoCompleteEngine::new(&pal_names)),
            pal_names,
            pal_api_url,
        })
    }

    // Fetches a Pal from the API.
    async fn get_pal(&self, pal: &str) -> Result<Pal, PalError> {
        let mut url = self.pal_api_url.clone();
        let query = format!("name={}", encode(pal));
        url.set_query(Some(&query));

        let response = reqwest::get(url).await.map_err(PalError::Reqwest)?;

        let parsed = match response.status() {
            reqwest::StatusCode::OK => response
                .json::<APIResponse>()
                .await
                .map_err(PalError::Reqwest),
            reqwest::StatusCode::UNAUTHORIZED => Err(PalError::TokenExpired),
            other => Err(PalError::Unexpected(anyhow!(
                "Unexpected status code: {}",
                other
            ))),
        }?;

        match parsed.content.first() {
            Some(pal) => Ok(pal.clone()),
            None => Err(PalError::MissingContent),
        }
    }
}

type Context<'a> = poise::Context<'a, State, anyhow::Error>;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct APIResponse {
    pub content: Vec<Pal>,
    pub page: i64,
    pub limit: i64,
    pub count: i64,
    pub total: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pal {
    pub id: i64,
    pub key: String,
    pub image: String,
    pub name: String,
    pub wiki: String,
    pub types: Vec<String>,
    pub image_wiki: String,
    pub suitability: Vec<Suitability>,
    pub drops: Vec<String>,
    pub aura: Aura,
    pub description: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PalError {
    #[error("No Pal named `{0}` was found")]
    NoPalFound(String),

    #[error("Error fetching from API: `{0}`")]
    Reqwest(reqwest::Error),

    #[error("Discord token is expired")]
    TokenExpired,

    #[error("Missing content in response from API")]
    MissingContent,

    #[error("Unexpected error: `{0}`")]
    Unexpected(anyhow::Error),
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suitability {
    #[serde(rename = "type")]
    pub type_field: String,
    pub level: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Aura {
    pub name: String,
    pub description: String,
}

// #[allow(clippy::unused_async)] // async required by poise `autocomplete` attribute macro
async fn autocomplete_pal<'a>(ctx: Context<'_>, partial: &'a str) -> Vec<String> {
    if partial.is_empty() {
        return ctx.data().pal_names.clone();
    }

    let ac_eng = ctx.data().ac_eng.clone();
    let partial = partial.to_owned();
    match tokio::task::spawn(async move { ac_eng.autocomplete(&partial) }).await {
        Ok(pals) => pals,
        Err(err) => {
            error!("Error fetching autocomplete: {err:?}");
            vec![]
        }
    }
}

/// Fetches the names of all Pals from the API.
async fn get_pal_names<T: IntoUrl + Display>(pal_api_url: &T) -> Result<Vec<String>> {
    let pal_names = reqwest::get(format!("{pal_api_url}?limit=200"))
        .await?
        .json::<APIResponse>()
        .await?
        .content
        .iter()
        .map(|c| c.name.clone())
        .collect::<Vec<_>>();

    Ok(pal_names)
}

/// Formats a name into a wiki link.
fn format_wiki(name: &str) -> String {
    let name = name.to_title_case();
    let url = name.replace(' ', "_");
    format!("[{name}](https://palworld.fandom.com/wiki/{url})")
}

/// Sends an error message to the channel from the original message.
async fn reply_with_error(ctx: &Context<'_>, error: &PalError) {
    match &error {
        PalError::NoPalFound(_) => {}
        err => {
            error!("{}", err);
        }
    }

    if let Err(why) = ctx.say(format!("**Error**: {error}")).await {
        error!("Error sending message: {why:?}");
    }
}

#[poise::command(prefix_command)]
async fn register(ctx: Context<'_>) -> Result<()> {
    debug!(
        "Registering application commands to {}#{}",
        if let Some(guild_id) = ctx.guild_id() {
            guild_id.name(ctx).unwrap_or("global".to_string())
        } else {
            "global".to_string()
        },
        ctx.channel_id().name(&ctx).await?
    );
    register_application_commands_buttons(ctx).await?;

    Ok(())
}

#[poise::command(slash_command)]
async fn pal(
    ctx: Context<'_>,

    #[description = "Pal"]
    #[autocomplete = "autocomplete_pal"]
    pal: String,
) -> Result<()> {
    let state = ctx.data();
    let pal = match state.get_pal(&pal).await {
        Ok(pal) => pal,
        Err(err) => {
            reply_with_error(&ctx, &err).await;
            return Err(err.into());
        }
    };

    let types = &pal
        .types
        .iter()
        .map(|typ| format_wiki(typ))
        .collect::<Vec<_>>()
        .join(", ");

    let suitabilities = &pal
        .suitability
        .iter()
        .map(|s| format!("* {} {}", format_wiki(&s.type_field), s.level))
        .collect::<Vec<_>>()
        .join("\n");

    let drops = &pal
        .drops
        .iter()
        .map(|drop| format!("* {}", format_wiki(drop)))
        .collect::<Vec<_>>()
        .join("\n");

    let aura_name = pal.aura.name.to_title_case();

    let embed = CreateEmbed::new()
        .title(&pal.name)
        .description(&pal.description)
        .thumbnail(&pal.image_wiki)
        .fields(vec![
            (
                "Number",
                format!("[#{}]({})", &pal.id, &pal.wiki).as_str(),
                true,
            ),
            (
                if pal.types.len() == 1 {
                    "Type"
                } else {
                    "Types"
                },
                types,
                true,
            ),
            (&aura_name, &pal.aura.description, false),
            (
                if pal.suitability.len() == 1 {
                    "Work Suitability"
                } else {
                    "Work Suitabilities"
                },
                &suitabilities,
                false,
            ),
            ("Drops", &drops, false),
        ]);

    ctx.send(CreateReply::default().embed(embed))
        .await
        .map(|_| ())
        .map_err(|err| {
            error!("Error sending message: {err:?}");
            err.into()
        })
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load env vars from .env, if available.
    let _ = dotenv();
    env_logger::init();

    let token = env::var("DISCORD_TOKEN").expect("Expected a DISCORD_TOKEN in the environment");
    let pal_api_url = env::var("PAL_API_URL").expect("Expected a PAL_API_URL in the environment");

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![pal(), register()],
            prefix_options: PrefixFrameworkOptions {
                prefix: Some("!".to_string()),
                ..Default::default()
            },
            ..Default::default()
        })
        .setup(|ctx, ready, framework| {
            Box::pin(async move {
                info!("{} is connected!", ready.user.name);
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                State::new(&pal_api_url).await
            })
        })
        .build();

    let intents = GatewayIntents::non_privileged()
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;

    let mut client = ClientBuilder::new(&token, intents)
        .framework(framework)
        .await
        .expect("Err creating client");

    client.start().await.map_err(anyhow::Error::from)
}
