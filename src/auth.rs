use std::{
    thread::sleep,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use reqwest::blocking::{Client, Response};
use serde::Deserialize;

use crate::{
    error::Result,
    store::{TokenFile, Tokens},
};

const CLIENT_ID: &str = "fob-cli";
const SCOPE: &str = "openid console offline_access";
const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

#[derive(Deserialize)]
struct Metadata {
    token_endpoint: String,
    device_authorization_endpoint: Option<String>,
    revocation_endpoint: Option<String>,
}

#[derive(Deserialize)]
struct Device {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: Option<String>,
    expires_in: u64,
    interval: Option<u64>,
}

#[derive(Deserialize)]
struct Granted {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: u64,
}

#[derive(Deserialize, Default)]
struct Refused {
    #[serde(default)]
    error: String,
    error_description: Option<String>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

fn refused(response: Response) -> Refused {
    let status = response.status();
    let mut refused: Refused = response.json().unwrap_or_default();
    if refused.error_description.is_none() {
        refused.error_description = Some(format!("Sign-in failed ({status})."));
    }
    refused
}

pub fn login(client: &Client, tenant: &str, org: &str, browser: bool) -> Result<()> {
    let response = client
        .get(format!("{tenant}/.well-known/oauth-authorization-server"))
        .send()?;
    if !response.status().is_success() {
        return Err(format!("No organization {org} at {tenant}. Check --org and --host.").into());
    }
    let metadata: Metadata = response.json()?;
    let device_endpoint = metadata
        .device_authorization_endpoint
        .ok_or("This server doesn't support signing in from the command line.")?;

    let response = client
        .post(device_endpoint)
        .form(&[("client_id", CLIENT_ID), ("scope", SCOPE)])
        .send()?;
    if !response.status().is_success() {
        return Err(refused(response)
            .error_description
            .unwrap_or_default()
            .into());
    }
    let device: Device = response.json()?;

    let url = device
        .verification_uri_complete
        .as_deref()
        .unwrap_or(&device.verification_uri);
    eprintln!("Open {url}\nCode: {}", device.user_code);
    if browser {
        let _ = open::that(url);
    }

    let deadline = Instant::now() + Duration::from_secs(device.expires_in);
    let mut interval = device.interval.unwrap_or(5).max(1);
    let granted: Granted = loop {
        sleep(Duration::from_secs(interval));
        if Instant::now() > deadline {
            return Err("The code expired. Run `fob login` again.".into());
        }
        let response = client
            .post(&metadata.token_endpoint)
            .form(&[
                ("grant_type", DEVICE_GRANT),
                ("device_code", &device.device_code),
                ("client_id", CLIENT_ID),
            ])
            .send()?;
        if response.status().is_success() {
            break response.json()?;
        }
        let refused = refused(response);
        match refused.error.as_str() {
            "authorization_pending" => {}
            "slow_down" => interval += 5,
            _ => return Err(refused.error_description.unwrap_or_default().into()),
        }
    };

    let mut file = TokenFile::lock()?;
    file.entries.insert(
        tenant.to_owned(),
        Tokens {
            access_token: granted.access_token,
            refresh_token: granted.refresh_token,
            expires_at: now() + granted.expires_in,
            token_endpoint: metadata.token_endpoint,
            revocation_endpoint: metadata.revocation_endpoint,
        },
    );
    file.save()
}

/// Whether there was a sign-in to end.
pub fn logout(client: &Client, tenant: &str) -> Result<bool> {
    let mut file = TokenFile::lock()?;
    let Some(tokens) = file.entries.remove(tenant) else {
        return Ok(false);
    };
    file.save()?;
    if let (Some(endpoint), Some(refresh)) = (tokens.revocation_endpoint, tokens.refresh_token) {
        let _ = client
            .post(endpoint)
            .form(&[("token", refresh.as_str()), ("client_id", CLIENT_ID)])
            .send();
    }
    Ok(true)
}

pub fn access_token(client: &Client, tenant: &str, org: &str) -> Result<String> {
    let sign_in = || format!("Run `fob login --org {org}`.");
    let mut file = TokenFile::lock()?;
    let Some(tokens) = file.entries.get(tenant).cloned() else {
        return Err(format!("Not signed in to {org}. {}", sign_in()).into());
    };
    if tokens.expires_at > now() + 30 {
        return Ok(tokens.access_token);
    }
    let ended = || format!("Your sign-in to {org} has ended. {}", sign_in());
    let Some(refresh) = tokens.refresh_token.clone() else {
        return Err(ended().into());
    };
    let response = client
        .post(&tokens.token_endpoint)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", &refresh),
        ])
        .send()?;
    if !response.status().is_success() {
        let refused = refused(response);
        if refused.error == "invalid_grant" {
            file.entries.remove(tenant);
            file.save()?;
            return Err(ended().into());
        }
        return Err(refused.error_description.unwrap_or_default().into());
    }
    let granted: Granted = response.json()?;
    let access_token = granted.access_token.clone();
    file.entries.insert(
        tenant.to_owned(),
        Tokens {
            access_token: granted.access_token,
            refresh_token: granted.refresh_token.or(tokens.refresh_token),
            expires_at: now() + granted.expires_in,
            ..tokens
        },
    );
    file.save()?;
    Ok(access_token)
}
