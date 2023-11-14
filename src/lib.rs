use async_openai_wasi::{
    types::{
        ChatCompletionFunctionsArgs, ChatCompletionRequestFunctionMessageArgs,
        ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
        ChatCompletionToolArgs, ChatCompletionToolType, CreateChatCompletionRequestArgs,
        FinishReason, Role,
    },
    Client,
};
use chrono::prelude::*;
use dotenv::dotenv;
use flowsnet_platform_sdk::{logger, write_error_log};
use http_req::{
    request::{Method, Request},
    uri::Uri,
};
use serde::Deserialize;
use serde_json::json;
use slack_flows::{listen_to_channel, send_message_to_channel};
use std::collections::HashMap;
use std::env;
use web_scraper_flows::get_page_text;

#[no_mangle]
#[tokio::main(flavor = "current_thread")]
async fn run() {
    logger::init();
    dotenv().ok();
    let slack_workspace = env::var("slack_workspace").unwrap_or("secondstate".to_string());
    let slack_channel = env::var("slack_channel").unwrap_or("test-flow".to_string());

    listen_to_channel(&slack_workspace, &slack_channel, |sm| {
        handler(&slack_workspace, &slack_channel, sm.text)
    })
    .await;
}

#[no_mangle]
async fn handler(workspace: &str, channel: &str, msg: String) {
    let trigger_word = env::var("trigger_word").unwrap_or("tool_calls".to_string());

    match msg.starts_with(&trigger_word) {
        false => {}

        true => {
            let user_input = msg.replace(&trigger_word, "").to_string();

            let _ = run_gpt(workspace, channel, user_input).await;
        }
    }
}

pub async fn run_gpt(
    workspace: &str,
    channel: &str,
    user_input: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new();
    let mut messages = vec![
        ChatCompletionRequestSystemMessageArgs::default()
            .content("Perform function requests for the user")
            .build()?
            .into(),
        ChatCompletionRequestUserMessageArgs::default()
            .content(user_input)
            // .content("Hello, I am a user, I would like to know the time of day now")
            .build()?
            .into(),
    ];

    let tools = vec![
        ChatCompletionToolArgs::default()
            .r#type(ChatCompletionToolType::Function)
            .function(
                ChatCompletionFunctionsArgs::default()
                    .name("getWeather")
                    .description("Get weather forecast for the city passed to it")
                    .parameters(json!({
                        "type": "object",
                        "properties": {
                            "city": {
                                "type": "string",
                                "description": "The city specified by the user",
                            },
                        },
                        "required": ["city"],
                    }))
                    .build()?,
            )
            .build()?,
        ChatCompletionToolArgs::default()
            .r#type(ChatCompletionToolType::Function)
            .function(
                ChatCompletionFunctionsArgs::default()
                    .name("scraper")
                    .description("Get the text content of the webpage from the url passed to it")
                    .parameters(json!({
                        "type": "object",
                        "properties": {
                            "url": {
                                "type": "string",
                                "description": "The url from which to fetch the content",
                            },
                        },
                        "required": ["url"],
                    }))
                    .build()?,
            )
            .build()?,
        ChatCompletionToolArgs::default()
            .r#type(ChatCompletionToolType::Function)
            .function(
                ChatCompletionFunctionsArgs::default()
                    .name("getTimeOfDay")
                    .description("Get the time of day.")
                    .parameters(json!({
                        "type": "object",
                        "properties": {},
                        "required": [],
                    }))
                    .build()?,
            )
            .build()?,
    ];

    let request = CreateChatCompletionRequestArgs::default()
        .max_tokens(512u16)
        .model("gpt-3.5-turbo-0613")
        .messages(messages.clone())
        .tools(tools)
        .build()?;

    let chat = client.chat().create(request).await?;

    let wants_to_use_function = chat
        .choices
        .get(0)
        .map(|choice| choice.finish_reason == Some(FinishReason::FunctionCall))
        .unwrap_or(false);

    log::info!("wants_to_use_function: {}", wants_to_use_function);

    if wants_to_use_function {
        let tool_calls = chat.choices[0].message.tool_calls.as_ref().unwrap();

        for tool_call in tool_calls {
            let function = &tool_call.function;
            let content_str = function.name.clone();
            log::info!("content_str: {}", content_str);

            let content = match function.name.as_str() {
                "getWeather" => {
                    let argument_obj =
                        serde_json::from_str::<HashMap<String, String>>(&function.arguments)?;

                        let city = &argument_obj["city"];
                        log::info!("city: {}", city);

                    get_weather(&argument_obj["city"].clone())
                }
                "scraper" => {
                    let argument_obj =
                        serde_json::from_str::<HashMap<String, String>>(&function.arguments)?;

                        let url = &argument_obj["url"];
                        log::info!("url: {}", url);
                        
                    scraper(argument_obj["url"].clone()).await
                }
                "getTimeOfDay" => get_time_of_day(),
                _ => "".to_string(),
            };
            messages.push(
                ChatCompletionRequestFunctionMessageArgs::default()
                    .role(Role::Function)
                    .name(function.name.clone())
                    .content(content)
                    .build()?
                    .into(),
            );
        }
    }

    let response_after_func_run = client
        .chat()
        .create(
            CreateChatCompletionRequestArgs::default()
                .model("gpt-3.5-turbo-0613")
                .messages(messages)
                .build()?,
        )
        .await?;

    let res = response_after_func_run
        .choices
        .get(0)
        .unwrap()
        .message
        .clone()
        .content
        .unwrap_or("no result".to_string());
    send_message_to_channel(workspace, channel, res).await;

    Ok(())
}

fn get_weather(city: &str) -> String {
    if let Some(w) = get_weather_inner(&city) {
        format!(
            r#"
Today in {}
{}
Low temperature: {} °C,
High temperature: {} °C,
Wind Speed: {} km/h"#,
            city,
            w.weather
                .first()
                .unwrap_or(&Weather {
                    main: "Unknown".to_string()
                })
                .main,
            w.main.temp_min as i32,
            w.main.temp_max as i32,
            w.wind.speed as i32
        )
    } else {
        String::from("No city or incorrect spelling")
    }
}

async fn scraper(url: String) -> String {
    match get_page_text(&url).await {
        Err(_e) => "failed to get webpage".to_string(),

        Ok(txt) => txt,
    }
}

fn get_time_of_day() -> String {
    let now = Local::now();
    now.to_rfc3339()
}

#[derive(Deserialize, Debug)]
struct ApiResult {
    weather: Vec<Weather>,
    main: Main,
    wind: Wind,
}

#[derive(Deserialize, Debug)]
struct Weather {
    main: String,
}

#[derive(Deserialize, Debug)]
struct Main {
    temp_max: f64,
    temp_min: f64,
}

#[derive(Deserialize, Debug)]
struct Wind {
    speed: f64,
}

fn get_weather_inner(city: &str) -> Option<ApiResult> {
    let mut writer = Vec::new();
    let api_key = env::var("API_KEY").unwrap_or("fake_api_key".to_string());
    let query_str = format!(
        "https://api.openweathermap.org/data/2.5/weather?q={city}&units=metric&appid={api_key}"
    );

    let uri = Uri::try_from(query_str.as_str()).unwrap();
    match Request::new(&uri).method(Method::GET).send(&mut writer) {
        Err(_e) => {}

        Ok(res) => {
            if !res.status_code().is_success() {
                return None;
            }
            match serde_json::from_slice::<ApiResult>(&writer) {
                Err(_e) => {}
                Ok(w) => {
                    return Some(w);
                }
            }
        }
    };
    None
}
