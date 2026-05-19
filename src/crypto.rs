use aes_gcm::{
    Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use rand_core::{OsRng, RngCore};
use serde::{Serialize, de::DeserializeOwned};
use std::{error::Error, fmt};

use crate::models::EncryptedPayload;

const AES_256_KEY_LEN: usize = 32;
const AES_GCM_NONCE_LEN: usize = 12;

#[derive(Debug)]
pub enum CryptoError {
    InvalidKeyLength { actual: usize },
    Decode,
    PayloadTooShort,
    Encrypt,
    Decrypt,
    Json,
}

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidKeyLength { actual } => {
                write!(f, "AES key must be 32 bytes, got {actual} bytes")
            }
            Self::Decode => write!(f, "encrypted payload is not valid base64"),
            Self::PayloadTooShort => write!(f, "encrypted payload is too short"),
            Self::Encrypt => write!(f, "encrypt payload failed"),
            Self::Decrypt => write!(f, "decrypt payload failed"),
            Self::Json => write!(f, "payload json encode/decode failed"),
        }
    }
}

impl Error for CryptoError {}

pub fn validate_aes_key(key: &str) -> Result<(), CryptoError> {
    if key.as_bytes().len() != AES_256_KEY_LEN {
        return Err(CryptoError::InvalidKeyLength {
            actual: key.as_bytes().len(),
        });
    }

    Ok(())
}

pub fn encrypt_json<T: Serialize>(value: &T, key: &str) -> Result<EncryptedPayload, CryptoError> {
    let plaintext = serde_json::to_vec(value).map_err(|_| CryptoError::Json)?;
    encrypt_bytes(&plaintext, key)
}

pub fn decrypt_json<T: DeserializeOwned>(
    payload: &EncryptedPayload,
    key: &str,
) -> Result<T, CryptoError> {
    let plaintext = decrypt_bytes(payload, key)?;
    serde_json::from_slice(&plaintext).map_err(|_| CryptoError::Json)
}

fn encrypt_bytes(plaintext: &[u8], key: &str) -> Result<EncryptedPayload, CryptoError> {
    validate_aes_key(key)?;

    let cipher = Aes256Gcm::new_from_slice(key.as_bytes()).map_err(|_| CryptoError::Encrypt)?;
    let mut nonce_bytes = [0_u8; AES_GCM_NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| CryptoError::Encrypt)?;

    let mut packed = Vec::with_capacity(AES_GCM_NONCE_LEN + ciphertext.len());
    packed.extend_from_slice(&nonce_bytes);
    packed.extend_from_slice(&ciphertext);

    Ok(EncryptedPayload {
        data: STANDARD.encode(packed),
    })
}

fn decrypt_bytes(payload: &EncryptedPayload, key: &str) -> Result<Vec<u8>, CryptoError> {
    validate_aes_key(key)?;

    let packed = STANDARD
        .decode(&payload.data)
        .map_err(|_| CryptoError::Decode)?;
    if packed.len() <= AES_GCM_NONCE_LEN {
        return Err(CryptoError::PayloadTooShort);
    }

    let (nonce_bytes, ciphertext) = packed.split_at(AES_GCM_NONCE_LEN);
    let cipher = Aes256Gcm::new_from_slice(key.as_bytes()).map_err(|_| CryptoError::Decrypt)?;
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|_| CryptoError::Decrypt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    const GOOD_KEY: &str = "12345678901234567890123456789012";
    const OTHER_KEY: &str = "abcdefghijklmnopqrstuvwxzy123456";

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct ExamplePayload {
        license_key: String,
        machine_code: String,
    }

    #[test]
    fn encrypted_json_can_be_decrypted_with_same_key() {
        let value = ExamplePayload {
            license_key: "LIC-test".to_string(),
            machine_code: "machine-001".to_string(),
        };

        let encrypted = encrypt_json(&value, GOOD_KEY).expect("encrypt");
        let decrypted: ExamplePayload = decrypt_json(&encrypted, GOOD_KEY).expect("decrypt");

        assert_eq!(decrypted, value);
    }

    #[test]
    fn decrypt_fails_with_wrong_key() {
        let value = ExamplePayload {
            license_key: "LIC-test".to_string(),
            machine_code: "machine-001".to_string(),
        };

        let encrypted = encrypt_json(&value, GOOD_KEY).expect("encrypt");

        assert!(decrypt_json::<ExamplePayload>(&encrypted, OTHER_KEY).is_err());
    }

    #[test]
    fn invalid_key_length_is_rejected() {
        let err = validate_aes_key("short").expect_err("short key should fail");

        assert!(matches!(err, CryptoError::InvalidKeyLength { actual: 5 }));
    }
}
