use anyhow::ensure;
use teloxide::{
    prelude::*,
    types::{ForwardedFrom, Recipient},
    utils::command::BotCommands,
};

mod redis;

const POWER_ON_TIME_KEY: &str = "power_on_time";
const WAKE_UP_TIME_KEY: &str = "wake_up_time";

#[derive(serde::Deserialize)]
struct EnvVariables {
    chat_id_to_report: i64,
    redis_address: String,
    bot_token: String,
    admin_user_id: u64,
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
enum BotCommand {
    #[command(description = "reply light status")]
    Status,
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
enum AdminCommand {
    Approve { user_id: u64 },
    Disapprove { user_id: u64 },
}

#[derive(Clone)]
struct BotEnv {
    redis: redis::RedisClient,
    admin_user_id: u64,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    pretty_env_logger::init();

    let env = envy::from_env::<EnvVariables>()?;

    let bot = Bot::new(env.bot_token);
    let mut redis_client = redis::RedisClient::connect(&env.redis_address)?;

    let is_admin = move |update: Message| {
        let admin_id = env.admin_user_id;
        let user_id = update.from().map(|user| user.id);
        let is_admin = user_id == Some(UserId(admin_id as u64));
        is_admin
    };

    let handler = Update::filter_message()
        .branch(
            dptree::entry()
                .filter_command::<BotCommand>()
                .endpoint(handler),
        )
        .branch(
            dptree::entry()
                .filter_command::<AdminCommand>()
                .filter(is_admin)
                .endpoint(admin_handler),
        )
        .branch(
            dptree::entry()
                .filter(|message: Message| {
                    let is_forward = message.forward_from().is_some();
                    is_forward
                })
                .filter(is_admin)
                .endpoint(forward_handler),
        );

    // The bot should notify how much time power was off
    // then it should reply to the message with the light status
    report_power_off_time(&bot, &mut redis_client, env.chat_id_to_report).await?;

    let env = BotEnv {
        redis: redis_client.clone(),
        admin_user_id: env.admin_user_id,
    };

    let mut dispatcher = Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![env])
        .build();
    let result = dispatcher.dispatch();

    futures::future::join(result, update_up_time(redis_client)).await;
    Ok(())
}

/// Reports how much time power was off on start up
async fn report_power_off_time(
    bot: &Bot,
    redis: &mut redis::RedisClient,
    chat_id: i64,
) -> anyhow::Result<()> {
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

    if !time_off.is_zero() && time_off < chrono::Duration::minutes(1) {
        bot.send_message(
            ChatId(chat_id),
            format!(
                "Less than 1 minute bot outage. Probably updating the bot. The power was on for {}\n",
                duration_formatter(time_light_was_on)
            ),
        )
        .await?;
        return Ok(());
    }

    bot.send_message(
        ChatId(chat_id),
        format!(
            "The power was off for {}.\nThe power was on for {}\n",
            duration_formatter(time_off),
            duration_formatter(time_light_was_on)
        ),
    )
    .await?;

    // Update wake up time
    redis.set(WAKE_UP_TIME_KEY, current_time)?;
    Ok(())
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

async fn admin_handler(
    bot: Bot,
    msg: Message,
    cmd: AdminCommand,
    bot_env: BotEnv,
) -> anyhow::Result<()> {
    match cmd {
        AdminCommand::Approve { user_id } => {
            bot_env.redis.approve_user(UserId(user_id))?;
            bot.send_message(ChatId::from(msg.chat.id), "Approved user")
                .send()
                .await?;
        }
        AdminCommand::Disapprove { user_id } => {
            bot_env.redis.disapprove_user(UserId(user_id))?;
            bot.send_message(ChatId::from(msg.chat.id), "Disapproved user")
                .send()
                .await?;
        }
    }

    Ok(())
}

async fn handler(bot: Bot, msg: Message, cmd: BotCommand, bot_env: BotEnv) -> anyhow::Result<()> {
    let user_id = msg
        .from()
        .map(|user| user.id)
        .ok_or_else(|| anyhow::anyhow!("Not a message"))?;
    // Permission check
    if !bot_env.redis.verify_approval(user_id)? && user_id.0 != bot_env.admin_user_id {
        bot.send_message(
            ChatId::from(msg.chat.id),
            "You are not entitled to use this command",
        )
        .send()
        .await?;
        return Ok(());
    }

    match cmd {
        BotCommand::Status => {
            let time = chrono::Utc::now();
            let msg_time = msg.date;

            if time - msg_time > chrono::Duration::minutes(1) {
                return Ok(()); // Ignore old messages, power was off
            }

            let stored_time = bot_env.redis.get(WAKE_UP_TIME_KEY).unwrap_or(time);

            let time_off = time - stored_time;
            if time_off == chrono::Duration::zero() {
                return Ok(()); // Shouldn't happen but just in case
            }
            let text = format!("Light is on for {}", duration_formatter(time_off));

            bot.send_message(msg.chat.id, text)
                .reply_to_message_id(msg.id)
                .send()
                .await?;
        }
    }
    Ok(())
}

async fn forward_handler(bot: Bot, msg: Message) -> anyhow::Result<()> {
    // Safe to unwrap because we only register this handler for forwarded messages
    let user_id = msg.forward().unwrap().from.clone();
    match user_id {
        ForwardedFrom::User(user) => {
            bot.send_message(msg.chat.id, format!("Forwarded from {}", user.id))
                .send()
                .await?
        }
        _ => {
            bot.send_message(msg.chat.id, "Can't get user id, ask user directly")
                .send()
                .await?
        }
    };
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
