use teloxide::{prelude::*, utils::command::BotCommands};

mod redis;

const POWER_ON_TIME_KEY: &str = "power_on_time";
const WAKE_UP_TIME_KEY: &str = "wake_up_time";

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
enum BotCommand {
    #[command(description = "reply light status")]
    Status,
}

#[async_trait::async_trait]
trait SendMessage {
    async fn send_msg(&self, text: String) -> anyhow::Result<()>;
}

#[async_trait::async_trait]
impl SendMessage for Bot {
    async fn send_msg(&self, text: String) -> anyhow::Result<()> {
        let chat_id = 373897581i64;
        self.send_message(ChatId(chat_id), text).await?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    pretty_env_logger::init();

    let bot = Bot::from_env();
    let mut redis_client = redis::RedisClient::connect();

    let handler = Update::filter_message().branch(
        dptree::entry()
            .filter_command::<BotCommand>()
            .endpoint(handler),
    );

    // The bot should notify how much time power was off
    // then it should reply to the message with the light status

    report_power_off_time(&bot, &mut redis_client).await;

    let redis_client2 = redis_client.clone();
    let mut dispatcher = Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![redis_client2])
        .build();
    let result = dispatcher.dispatch();

    futures::future::join(result, update_up_time(redis_client)).await;
    Ok(())
}

/// Reports how much time power was off on start up
async fn report_power_off_time(bot: &dyn SendMessage, redis: &mut redis::RedisClient) {
    let stored_time = redis
        .get(POWER_ON_TIME_KEY)
        .unwrap_or_else(|_| chrono::Utc::now());

    let current_time = chrono::Utc::now();
    redis
        .set(WAKE_UP_TIME_KEY, current_time)
        .expect("Failed to set wake up time");
    let time_off = current_time - stored_time;
    bot.send_msg(format!(
        "The power was off for {}",
        duration_formatter(time_off)
    ))
    .await
    .expect("Failed to send message");
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

            let stored_time = redis_client.get(WAKE_UP_TIME_KEY).unwrap_or_else(|_| time);

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
        result.push_str(&format!("{} days ", days));
    }
    let hours = duration.num_hours() % 24;
    if hours > 0 {
        result.push_str(&format!("{} hours ", hours));
    }
    let minutes = duration.num_minutes() % 60;
    if minutes > 0 {
        result.push_str(&format!("{} minutes ", minutes));
    }
    let seconds = duration.num_seconds() % 60;
    if seconds > 0 {
        result.push_str(&format!("{} seconds ", seconds));
    }
    result
}
