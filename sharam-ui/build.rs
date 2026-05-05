use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=../Sharam.toml");
    println!("cargo:rerun-if-env-changed=SHARAM_GOOGLE__CLIENT_ID");

    if let Ok(env_val) = std::env::var("SHARAM_GOOGLE__CLIENT_ID") {
        println!("cargo:rustc-env=SHARAM_GOOGLE_CLIENT_ID={env_val}");
        return;
    }

    let toml_path = Path::new("../Sharam.toml");
    if !toml_path.exists() {
        println!(
            "cargo:warning=Sharam.toml not found and SHARAM_GOOGLE__CLIENT_ID env var not set; \
             Google sign-in will be disabled until one is provided."
        );
        return;
    }

    let raw = std::fs::read_to_string(toml_path).expect("read Sharam.toml");
    let parsed: toml::Value = toml::from_str(&raw).expect("parse Sharam.toml");
    let client_id = parsed
        .get("google")
        .and_then(|g| g.get("client_id"))
        .and_then(|v| v.as_str())
        .expect("[google].client_id missing in Sharam.toml");

    println!("cargo:rustc-env=SHARAM_GOOGLE_CLIENT_ID={client_id}");
}
