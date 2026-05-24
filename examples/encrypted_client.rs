use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{env, error::Error};

const NONCE_LEN: usize = 12;

#[derive(Debug, Serialize, Deserialize)]
struct EncryptedPayload {
    data: String,
}

#[derive(Debug, Serialize)]
struct VerifyRequest {
    license_key: String,
    machine_code: String,
}

#[derive(Debug, Deserialize)]
struct VerifyResponse {
    status: String,
    message: String,
    expires_at: Option<String>,
    license_type: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let api_url = env::var("VERIFY_API_URL")
        .unwrap_or_else(|_| "http://1.15.171.108:3000/api/verify".to_string());
    let api_key =
        env::var("APP_API_KEY").unwrap_or_else(|_| "901F0CF02D2B48D19966A4947E884398".to_string());
    let client_aes_key = env::var("CLIENT_AES_KEY")
        .unwrap_or_else(|_| "E0DC0CC761604122A06648A4BE9893EE".to_string());
    let server_aes_key = env::var("SERVER_AES_KEY")
        .unwrap_or_else(|_| "CBD0782890A041E2A8666D9668C43F25".to_string());

    let request = VerifyRequest {
        license_key: env::var("LICENSE_KEY")
            .unwrap_or_else(|_| "LIC-f55f3d691ef24774b60d6c12eb4576ac".to_string()),
        machine_code: env::var("MACHINE_CODE").unwrap_or_else(|_| "machine-001".to_string()),
    };

    let encrypted_request = encrypt_json(&request, &client_aes_key)?;
    let encrypted_response = reqwest::Client::new()
        .post(api_url)
        .header("X-Api-Key", api_key)
        .json(&encrypted_request)
        .send()
        .await?
        .error_for_status()?
        .json::<EncryptedPayload>()
        .await?;

    let response: VerifyResponse = decrypt_json(&encrypted_response, &server_aes_key)?;
    println!(
        "status={} message={} expires_at={} license_type={}",
        response.status,
        response.message,
        response.expires_at.as_deref().unwrap_or("null"),
        response.license_type.as_deref().unwrap_or("null")
    );

    Ok(())
}

fn encrypt_json<T: Serialize>(value: &T, key: &str) -> Result<EncryptedPayload, Box<dyn Error>> {
    validate_key(key)?;

    let plaintext = serde_json::to_vec(value)?;
    let cipher = Aes256Gcm::new_from_slice(key.as_bytes()).map_err(|_| "invalid AES key")?;
    let mut nonce_bytes = [0_u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_ref())
        .map_err(|_| "encrypt failed")?;

    let mut packed = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    packed.extend_from_slice(&nonce_bytes);
    packed.extend_from_slice(&ciphertext);

    Ok(EncryptedPayload {
        data: STANDARD.encode(packed),
    })
}

fn decrypt_json<T: DeserializeOwned>(
    payload: &EncryptedPayload,
    key: &str,
) -> Result<T, Box<dyn Error>> {
    validate_key(key)?;

    let packed = STANDARD.decode(&payload.data)?;
    if packed.len() <= NONCE_LEN {
        return Err("encrypted payload is too short".into());
    }

    let (nonce_bytes, ciphertext) = packed.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new_from_slice(key.as_bytes()).map_err(|_| "invalid AES key")?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|_| "decrypt failed")?;

    Ok(serde_json::from_slice(&plaintext)?)
}

fn validate_key(key: &str) -> Result<(), Box<dyn Error>> {
    if key.as_bytes().len() != 32 {
        return Err(format!(
            "AES key must be 32 bytes, got {} bytes",
            key.as_bytes().len()
        )
        .into());
    }

    Ok(())
}
