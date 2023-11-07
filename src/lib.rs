// use dotenv;
use flowsnet_platform_sdk::logger;
use http_req::{
    request::{Method, Request},
    uri::Uri,
};
use openai_flows::{
    chat::{ChatModel, ChatOptions},
    OpenAIFlows,
};
use serde::Deserialize;
use serde_json;
use serde_json::json;
use std::env;
use store_flows::{get, set, Expire, ExpireKind};
use tg_flows::{listen_to_update, update_handler, Telegram, Update, UpdateKind};

#[no_mangle]
#[tokio::main(flavor = "current_thread")]
pub async fn on_deploy() {
    let telegram_token = std::env::var("telegram_token").unwrap();

    let _forecast = match get_weather("Toronto") {
        Some(w) => format!(
            r#"
Today in {}
{}
Low temperature: {} °C,
High temperature: {} °C,
Wind Speed: {} km/h"#,
            "Toronto",
            w.weather
                .first()
                .unwrap_or(&Weather {
                    main: "Unknown".to_string()
                })
                .main,
            w.main.temp_min as i32,
            w.main.temp_max as i32,
            w.wind.speed as i32
        ),
        None => String::from("No city or incorrect spelling"),
    };
    set(
        "_weather",
        serde_json::json!(_forecast),
        Some(Expire {
            kind: ExpireKind::Ex,
            value: 300,
        }),
    );
    listen_to_update(telegram_token).await;
}

#[update_handler]
async fn handler(update: Update) {
    logger::init();
    let telegram_token = std::env::var("telegram_token").unwrap();
    let placeholder_text = std::env::var("placeholder").unwrap_or("Typing ...".to_string());
    let system_prompt = std::env::var("system_prompt")
        .unwrap_or("You are a helpful assistant answering questions on Telegram.".to_string());
    let help_mesg = std::env::var("help_mesg").unwrap_or("I am your assistant on Telegram. Ask me any question! To start a new conversation, type the /restart command.".to_string());

    let tele = Telegram::new(telegram_token.to_string());

    if let UpdateKind::Message(msg) = update.kind {
        let chat_id = msg.chat.id;
        log::info!("Received message from {}", chat_id);

        let mut openai = OpenAIFlows::new();
        openai.set_retry_times(3);
        let mut co = ChatOptions::default();
        // co.model = ChatModel::GPT4;
        co.model = ChatModel::GPT35Turbo16K;
        co.restart = false;
        co.system_prompt = Some(&system_prompt);

        let text = msg.text().unwrap_or("");
        if text.eq_ignore_ascii_case("/help") {
            _ = tele.send_message(chat_id, &help_mesg);
        } else if text.eq_ignore_ascii_case("/weather") {
            let _forecast = match get("_forecast") {
                Some(v) => v.as_str().unwrap_or("Invalid data").to_string(),
                None => String::from("no forecast data"),
            };

            _ = tele.send_message(chat_id, &_forecast);
        } else if text.eq_ignore_ascii_case("/start") {
            _ = tele.send_message(chat_id, &help_mesg);
            set(&chat_id.to_string(), json!(true), None);
            log::info!("Started converstion for {}", chat_id);
        } else if text.eq_ignore_ascii_case("/restart") {
            _ = tele.send_message(chat_id, "Ok, I am starting a new conversation.");
            set(&chat_id.to_string(), json!(true), None);
            log::info!("Restarted converstion for {}", chat_id);
        } else {
            let placeholder = tele
                .send_message(chat_id, &placeholder_text)
                .expect("Error occurs when sending Message to Telegram");

            let restart = match get(&chat_id.to_string()) {
                Some(v) => v.as_bool().unwrap_or_default(),
                None => false,
            };
            if restart {
                log::info!("Detected restart = true");
                set(&chat_id.to_string(), json!(false), None);
                co.restart = true;
            }

            match openai
                .chat_completion(&chat_id.to_string(), &text, &co)
                .await
            {
                Ok(r) => {
                    _ = tele.edit_message_text(chat_id, placeholder.id, r.choice);
                }
                Err(e) => {
                    _ = tele.edit_message_text(
                        chat_id,
                        placeholder.id,
                        "Sorry, an error has occured. Please try again later!",
                    );
                    log::error!("OpenAI returns error: {}", e);
                }
            }
        }
    }
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

fn get_weather(city: &str) -> Option<ApiResult> {
    let mut writer = Vec::new();
    let api_key = env::var("API_KEY").unwrap_or("fake_api_key".to_string());
    let query_str = format!(
        "https://api.openweathermap.org/data/2.5/weather?q={city}&units=metric&appid={api_key}"
    );

    let uri = Uri::try_from(query_str.as_str()).unwrap();
    match Request::new(&uri).method(Method::GET).send(&mut writer) {
        Err(_e) => log::error!("Error getting response from weather api: {:?}", _e),

        Ok(res) => {
            if !res.status_code().is_success() {
                log::error!("weather api http error: {:?}", res.status_code());
                return None;
            }
            match serde_json::from_slice::<ApiResult>(&writer) {
                Err(_e) => log::error!("Error deserializing weather api response: {:?}", _e),
                Ok(w) => {
                    return Some(w);
                }
            }
        }
    };
    None
}
