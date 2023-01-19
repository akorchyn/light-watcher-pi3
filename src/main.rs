use teloxide::{prelude::*, utils::command::BotCommands};

mod redis;

const POWER_ON_TIME_KEY: &str = "power_on_time";
const WAKE_UP_TIME_KEY: &str = "wake_up_time";

#[derive(serde::Deserialize)]
struct EnvVariables {
    chat_id_to_report: i64,
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
enum BotCommand {
    #[command(description = "reply light status")]
    Status,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    pretty_env_logger::init();

    let env = envy::from_env::<EnvVariables>()?;

    let bot = Bot::from_env();
    let mut redis_client = redis::RedisClient::connect();

    let handler = Update::filter_message().branch(
        dptree::entry()
            .filter_command::<BotCommand>()
            .endpoint(handler),
    );

    // The bot should notify how much time power was off
    // then it should reply to the message with the light status

    report_power_off_time(&bot, &mut redis_client, env.chat_id_to_report).await;

    let redis_client2 = redis_client.clone();
    let mut dispatcher = Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![redis_client2])
        .build();
    let result = dispatcher.dispatch();

    futures::future::join(result, update_up_time(redis_client)).await;
    Ok(())
}

/// Reports how much time power was off on start up
async fn report_power_off_time(bot: &Bot, redis: &mut redis::RedisClient, chat_id: i64) {
    let stored_time = redis
        .get(POWER_ON_TIME_KEY)
        .unwrap_or_else(|_| chrono::Utc::now());
    let wake_up_time: chrono::DateTime<chrono::Utc> = redis
        .get(WAKE_UP_TIME_KEY)
        .unwrap_or_else(|_| chrono::Utc::now());

    let current_time = chrono::Utc::now();
    let time_until_wake_up = current_time - wake_up_time;
    let time_off = current_time - stored_time;
    let time_light_was_on = time_until_wake_up - time_off;

    if time_off < chrono::Duration::minutes(1) {
        bot.send_message(
            ChatId(chat_id),
            format!(
                "Less than 1 minute bot outage. Probably updating the bot. The power was on for {}\n",
                duration_formatter(time_light_was_on)
            ),
        )
        .await
        .expect("Failed to send message");
        return;
    }

    bot.send_message(
        ChatId(chat_id),
        format!(
            "The power was off for {}\n. The power was on for {}\n",
            duration_formatter(time_off),
            duration_formatter(time_light_was_on)
        ),
    )
    .await
    .expect("Failed to send message");

    // Update wake up time
    redis
        .set(WAKE_UP_TIME_KEY, current_time)
        .expect("Failed to set wake up time");
}

/// Updates that power is on every minute
async fn update_up_time(redis_client: redis::RedisClient) {
    let sleep_duration = std::time::Duration::from_secs(60);
    loop {
        tokio::time::sleep(sleep_duration).await;

        let current_time = chrono::Utc::now();
        let err: anyhow::Result<()> = redis_client.set(POWER_ON_TIME_KEY, current_time);
        if err.is_err() {
            continue;
        }
    }
}

async fn handler(
    bot: Bot,
    msg: Message,
    cmd: BotCommand,
    redis_client: redis::RedisClient,
) -> ResponseResult<()> {
    match cmd {
        BotCommand::Status => {
            let time = chrono::Utc::now();
            let msg_time = msg.date;

            if time - msg_time > chrono::Duration::minutes(1) {
                return Ok(()); // Ignore old messages, power was off
            }

            let stored_time = redis_client.get(WAKE_UP_TIME_KEY).unwrap_or(time);

            let time_off = time - stored_time;
            let text = format!("Light is on for {}", duration_formatter(time_off));

            bot.send_message(msg.chat.id, text)
                .reply_to_message_id(msg.id)
                .send()
                .await?;
        }
    }
    Ok(())
}

fn duration_formatter(duration: chrono::Duration) -> String {
    let mut result = String::new();
    let days = duration.num_days();
    if days > 0 {
        result.push_str(&format!("{days} days "));
    }
    let hours = duration.num_hours() % 24;
    if hours > 0 {
        result.push_str(&format!("{hours} hours "));
    }
    let minutes = duration.num_minutes() % 60;
    if minutes > 0 {
        result.push_str(&format!("{minutes} minutes "));
    }
    let seconds = duration.num_seconds() % 60;
    if seconds > 0 {
        result.push_str(&format!("{seconds} seconds "));
    }
    result
}
