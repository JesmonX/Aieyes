//! Release-only verifier: ensures installers are signed by the public key embedded in the app.
use base64::{Engine, engine::general_purpose::STANDARD};
use minisign_verify::{PublicKey, Signature};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("Expected installer path")?;
    let version = args.next().ok_or("Expected version")?;
    let key =
        String::from_utf8(STANDARD.decode(std::env::var("AIEYES_UPDATER_PUBLIC_KEY")?.trim())?)?;
    let signature_text = std::fs::read_to_string(format!("{path}.sig"))?;
    let signature =
        Signature::decode(&String::from_utf8(STANDARD.decode(signature_text.trim())?)?)?;
    PublicKey::decode(&key)?.verify(&std::fs::read(path)?, &signature, true)?;
    let marker = format!("version:{version}");
    if !signature
        .trusted_comment()
        .split_whitespace()
        .any(|word| word == marker)
    {
        return Err("Signature version does not match release".into());
    }
    println!("Update signature and version verified");
    Ok(())
}
