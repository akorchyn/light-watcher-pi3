use redis::Commands;
use teloxide::types::UserId;

#[derive(Clone)]
pub struct RedisClient {
    client: redis::Client,
}

impl RedisClient {
    pub fn connect(redis_addr: &str) -> anyhow::Result<Self> {
        let redis_client = redis::Client::open(redis_addr)?;

        Ok(Self {
            client: redis_client,
        })
    }

    pub fn get(&self, key: &str) -> Result<chrono::DateTime<chrono::Utc>, anyhow::Error> {
        let mut connection = self.client.get_connection()?;
        let value: String = connection.get(key)?;
        let time = chrono::DateTime::parse_from_rfc3339(&value)?;
        // Convert fixed offset to UTC
        let time = chrono::DateTime::<chrono::Utc>::from_utc(time.naive_utc(), chrono::Utc);
        Ok(time)
    }

    pub fn set(
        &self,
        key: &str,
        value: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), anyhow::Error> {
        let mut connection = self.client.get_connection()?;

        let value = value.to_rfc3339();
        connection.set(key, value)?;
        Ok(())
    }

    pub fn verify_approval(&self, user_id: UserId) -> Result<bool, anyhow::Error> {
        let mut connection = self.client.get_connection()?;
        let value: Option<String> = connection.get(user_id.to_string())?;
        Ok(value == Some("approved".to_string()))
    }

    fn manage_user(&self, user_id: UserId, value: &str) -> Result<(), anyhow::Error> {
        let mut connection = self.client.get_connection()?;
        connection.set(user_id.to_string(), value)?;
        Ok(())
    }

    pub fn approve_user(&self, user_id: UserId) -> Result<(), anyhow::Error> {
        self.manage_user(user_id, "approved")
    }

    pub fn disapprove_user(&self, user_id: UserId) -> Result<(), anyhow::Error> {
        self.manage_user(user_id, "disapproved")
    }
}
