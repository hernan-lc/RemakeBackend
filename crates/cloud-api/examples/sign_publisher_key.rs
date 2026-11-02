//! Sign a Cloud enrollment challenge locally with an existing TikTools private key.
use base64::{engine::general_purpose::STANDARD, Engine};
use std::{error::Error, path::Path};
use tiktools_plugin_package::SigningKey;
use uuid::Uuid;
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let [key_path, publisher, challenge, nonce] = args.as_slice() else {
        return Err(
            "usage: sign_publisher_key KEY_FILE PUBLISHER_UUID CHALLENGE_UUID NONCE".into(),
        );
    };
    let publisher = Uuid::parse_str(publisher)?;
    let challenge = Uuid::parse_str(challenge)?;
    if nonce.len() != 64
        || !nonce
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err("invalid challenge nonce".into());
    }
    let key = SigningKey::load(Path::new(key_path))?;
    let public = key.public_key();
    let message = format!(
        "tiktools-cloud-publisher-key-v1\n{publisher}\n{challenge}\n{nonce}\n{}\n{}",
        public.key_id, public.public_key
    );
    let output = serde_json::json!({"challengeId":challenge,"keyId":public.key_id,"publicKey":public.public_key,"signature":STANDARD.encode(key.sign(message.as_bytes()))});
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}
