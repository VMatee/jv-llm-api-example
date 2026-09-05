use crate::{Error, JvClient, Result, client::read_json, error::http_error};
use reqwest::{Method, header::HeaderValue};
use serde::Deserialize;
use zeroize::Zeroizing;

#[derive(Deserialize)]
struct LoginResponse {
    access_token: String,
}

impl JvClient {
    /// Store a temporary bearer in memory; never return or print login secrets.
    pub async fn login(&mut self, username: &str, password: &str) -> Result<()> {
        if self.is_authenticated() {
            return Err(Error::AlreadyAuthenticated);
        }
        if username.is_empty() || password.is_empty() {
            return Err(Error::InvalidInput("username and password are required"));
        }
        let response = self.http.post(self.endpoint("/v1/auth/login")?)
            .json(&serde_json::json!({"username": username, "password": password, "remember_me": false}))
            .send().await.map_err(|_| Error::Network)?;
        if response.status() != 200 {
            return Err(http_error(&response));
        }
        let payload: LoginResponse = read_json(response).await?;
        let token = Zeroizing::new(payload.access_token);
        if token.is_empty() || !token.bytes().all(|c| c.is_ascii_graphic()) {
            return Err(Error::MalformedResponse);
        }
        let bearer = Zeroizing::new(format!("Bearer {}", token.as_str()));
        let mut value = HeaderValue::from_str(&bearer).map_err(|_| Error::MalformedResponse)?;
        value.set_sensitive(true);
        self.authorization = Some(value);
        Ok(())
    }

    /// Attempt server revocation and always forget the local bearer, even on error.
    pub async fn logout(&mut self) -> Result<()> {
        let Some(token) = self.authorization.take() else {
            return Ok(());
        };
        let response = self
            .http
            .request(Method::POST, self.endpoint("/v1/auth/logout")?)
            .header("Authorization", token)
            .send()
            .await
            .map_err(|_| Error::Network)?;
        if response.status() != 204 {
            return Err(http_error(&response));
        }
        Ok(())
    }
}
