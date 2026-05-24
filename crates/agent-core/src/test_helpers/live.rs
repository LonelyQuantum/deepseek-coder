use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

pub const LIVE_TEST_FLAG: &str = "DEEPSEEK_CODER_LIVE_TESTS";
pub const LIVE_API_KEY_FILE: &str = ".secrets/deepseek-api-key";
pub const API_KEY_PLACEHOLDER: &str = "<put-your-deepseek-api-key-here>";

const API_KEY_ENV_VARS: [&str; 2] = ["DEEPSEEK_CODER_API_KEY", "DEEPSEEK_API_KEY"];

pub fn repo_root_from_crate_manifest(manifest_dir: impl AsRef<Path>) -> PathBuf {
    manifest_dir
        .as_ref()
        .parent()
        .and_then(|crates_dir| crates_dir.parent())
        .expect("crate manifest must be nested under crates/")
        .to_path_buf()
}

pub fn live_api_key(workspace_root: impl AsRef<Path>) -> io::Result<String> {
    for variable in API_KEY_ENV_VARS {
        if let Ok(api_key) = env::var(variable)
            && let Some(api_key) = normalize_api_key(&api_key)
        {
            return Ok(api_key);
        }
    }

    let api_key_path = workspace_root.as_ref().join(LIVE_API_KEY_FILE);
    let api_key = fs::read_to_string(&api_key_path).map_err(|source| {
        io::Error::new(
            source.kind(),
            format!(
                "{} and {} are not set and {} could not be read: {source}",
                API_KEY_ENV_VARS[0], API_KEY_ENV_VARS[1], LIVE_API_KEY_FILE
            ),
        )
    })?;
    normalize_api_key(&api_key).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "put a DeepSeek API key in {LIVE_API_KEY_FILE} or set {} / {}",
                API_KEY_ENV_VARS[0], API_KEY_ENV_VARS[1]
            ),
        )
    })
}

fn normalize_api_key(api_key: &str) -> Option<String> {
    let api_key = api_key.trim();
    if api_key.is_empty() || api_key == API_KEY_PLACEHOLDER {
        None
    } else {
        Some(api_key.to_owned())
    }
}
