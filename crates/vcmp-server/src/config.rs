use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use vcmp_protocol::DEFAULT_PORT;

pub const ENV_CONFIG: &str = "VCMP_SERVER_CONFIG";
pub const ENV_NAME: &str = "VCMP_SERVER_NAME";
pub const ENV_LISTEN: &str = "VCMP_SERVER_LISTEN";
pub const ENV_DATA: &str = "VCMP_SERVER_DATA";
pub const ENV_MUSIC: &str = "VCMP_SERVER_MUSIC";
pub const ENV_USER_TOKEN: &str = "VCMP_SERVER_USER_TOKEN";
pub const ENV_ADMIN_TOKEN: &str = "VCMP_SERVER_ADMIN_TOKEN";
pub const ENV_USER_TOKEN_FILE: &str = "VCMP_SERVER_USER_TOKEN_FILE";
pub const ENV_ADMIN_TOKEN_FILE: &str = "VCMP_SERVER_ADMIN_TOKEN_FILE";
pub const ENV_RESCAN_MINUTES: &str = "VCMP_SERVER_RESCAN_MINUTES";

pub const DEFAULT_RESCAN_MINUTES: u64 = 60;

pub const MIN_TOKEN_LENGTH: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub name: String,
    pub listen: SocketAddr,
    pub data: PathBuf,
    pub music: Vec<PathBuf>,
    pub user_token: Option<String>,
    pub admin_token: Option<String>,
    pub rescan_interval: Option<Duration>,
}

impl Config {
    pub fn new(data: impl Into<PathBuf>) -> Self {
        Self {
            name: "VCMP server".into(),
            listen: SocketAddr::from(([0, 0, 0, 0], DEFAULT_PORT)),
            data: data.into(),
            music: Vec::new(),
            user_token: None,
            admin_token: None,
            rescan_interval: Some(Duration::from_secs(DEFAULT_RESCAN_MINUTES * 60)),
        }
    }

    pub fn database(&self) -> PathBuf {
        self.data.join("library.sqlite3")
    }

    pub fn cache(&self) -> PathBuf {
        self.data.join("cache")
    }

    pub fn load(
        file: Option<&Path>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ConfigError> {
        let file = file
            .map(Path::to_path_buf)
            .or_else(|| env(ENV_CONFIG).map(PathBuf::from));
        let from_file = match &file {
            Some(path) => {
                let text = std::fs::read_to_string(path)
                    .map_err(|error| ConfigError::File(path.clone(), error.to_string()))?;
                toml::from_str::<FileConfig>(&text)
                    .map_err(|error| ConfigError::File(path.clone(), error.to_string()))?
            }
            None => FileConfig::default(),
        };

        let data = env(ENV_DATA)
            .map(PathBuf::from)
            .or(from_file.data)
            .unwrap_or_else(|| PathBuf::from("vcmp-server-data"));
        let mut config = Config::new(data);
        if let Some(name) = env(ENV_NAME).or(from_file.name) {
            let name = name.trim();
            if name.is_empty() {
                return Err(ConfigError::Invalid("name".into(), "it is empty".into()));
            }
            config.name = name.to_owned();
        }
        if let Some(listen) = env(ENV_LISTEN).or(from_file.listen) {
            config.listen = listen.parse().map_err(|_| {
                ConfigError::Invalid(
                    "listen".into(),
                    format!("{listen:?} is not an address and port such as 0.0.0.0:{DEFAULT_PORT}"),
                )
            })?;
        }
        config.music = match env(ENV_MUSIC) {
            Some(list) => std::env::split_paths(&list)
                .filter(|path| !path.as_os_str().is_empty())
                .collect(),
            None => from_file.music,
        };
        config.user_token = token(
            "user_token",
            env(ENV_USER_TOKEN),
            env(ENV_USER_TOKEN_FILE),
            from_file.user_token,
        )?;
        config.admin_token = token(
            "admin_token",
            env(ENV_ADMIN_TOKEN),
            env(ENV_ADMIN_TOKEN_FILE),
            from_file.admin_token,
        )?;
        if config.user_token.is_some() && config.user_token == config.admin_token {
            return Err(ConfigError::Invalid(
                "admin_token".into(),
                "it is the same as the user token".into(),
            ));
        }
        let minutes = match env(ENV_RESCAN_MINUTES) {
            Some(text) => text.trim().parse::<u64>().map_err(|_| {
                ConfigError::Invalid(
                    "rescan_minutes".into(),
                    format!("{text:?} is not a whole number"),
                )
            })?,
            None => from_file.rescan_minutes.unwrap_or(DEFAULT_RESCAN_MINUTES),
        };
        config.rescan_interval = (minutes > 0).then(|| Duration::from_secs(minutes * 60));
        Ok(config)
    }
}

fn token(
    name: &str,
    env: Option<String>,
    env_file: Option<String>,
    file: Option<String>,
) -> Result<Option<String>, ConfigError> {
    let value = match (env, env_file) {
        (Some(value), _) => Some(value),
        (None, Some(path)) => Some(
            std::fs::read_to_string(&path)
                .map_err(|error| ConfigError::File(PathBuf::from(&path), error.to_string()))?,
        ),
        (None, None) => file,
    };
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim().to_owned();
    if value.len() < MIN_TOKEN_LENGTH {
        return Err(ConfigError::Invalid(
            name.into(),
            format!("a token needs at least {MIN_TOKEN_LENGTH} characters"),
        ));
    }
    if !value.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(ConfigError::Invalid(
            name.into(),
            "a token may hold only visible ASCII characters, no spaces".into(),
        ));
    }
    Ok(Some(value))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    name: Option<String>,
    listen: Option<String>,
    data: Option<PathBuf>,
    #[serde(default)]
    music: Vec<PathBuf>,
    user_token: Option<String>,
    admin_token: Option<String>,
    rescan_minutes: Option<u64>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("the configuration file {0} can't be read: {1}")]
    File(PathBuf, String),
    #[error("{0} in the configuration is not valid: {1}")]
    Invalid(String, String),
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn defaults_without_a_file_or_environment() {
        let config = Config::load(None, env(&[])).unwrap();
        assert_eq!(config, Config::new("vcmp-server-data"));
        assert_eq!(config.listen.port(), DEFAULT_PORT);
        assert_eq!(config.rescan_interval, Some(Duration::from_secs(3600)));
    }

    #[test]
    fn the_environment_wins_over_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("vcmp-server.toml");
        std::fs::write(
            &file,
            r#"
name = "From the file"
listen = "127.0.0.1:9000"
data = "/srv/vcmp"
music = ["/music/a", "/music/b"]
user_token = "file-user-token-0123456789"
rescan_minutes = 15
"#,
        )
        .unwrap();
        let secret = dir.path().join("admin-secret");
        std::fs::write(&secret, "secret-admin-token-0123456789\n").unwrap();
        let config = Config::load(
            Some(&file),
            env(&[
                (ENV_NAME, "Home"),
                (ENV_ADMIN_TOKEN_FILE, secret.to_str().unwrap()),
                (ENV_RESCAN_MINUTES, "0"),
            ]),
        )
        .unwrap();
        assert_eq!(config.name, "Home");
        assert_eq!(config.listen, "127.0.0.1:9000".parse().unwrap());
        assert_eq!(config.data, PathBuf::from("/srv/vcmp"));
        assert_eq!(
            config.music,
            [PathBuf::from("/music/a"), PathBuf::from("/music/b")]
        );
        assert_eq!(
            config.user_token.as_deref(),
            Some("file-user-token-0123456789")
        );
        assert_eq!(
            config.admin_token.as_deref(),
            Some("secret-admin-token-0123456789")
        );
        assert_eq!(config.rescan_interval, None);
    }

    #[test]
    fn weak_or_equal_tokens_and_bad_values_are_refused() {
        assert!(matches!(
            Config::load(None, env(&[(ENV_USER_TOKEN, "short")])),
            Err(ConfigError::Invalid(name, _)) if name == "user_token"
        ));
        assert!(matches!(
            Config::load(None, env(&[(ENV_USER_TOKEN, "has a space in it, sadly")])),
            Err(ConfigError::Invalid(..))
        ));
        let same = "the-same-token-for-both";
        assert!(matches!(
            Config::load(None, env(&[(ENV_USER_TOKEN, same), (ENV_ADMIN_TOKEN, same)])),
            Err(ConfigError::Invalid(name, _)) if name == "admin_token"
        ));
        assert!(Config::load(None, env(&[(ENV_LISTEN, "everywhere")])).is_err());
        assert!(Config::load(None, env(&[(ENV_RESCAN_MINUTES, "soon")])).is_err());
        assert!(Config::load(None, env(&[(ENV_CONFIG, "/no/such/file.toml")])).is_err());
    }
}
