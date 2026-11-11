use std::sync::RwLock;

use vcmp_library::Library;
use vcmp_protocol::Role;

const SETTING_SERVER_ID: &str = "server.id";
const SETTING_USER_TOKEN: &str = "server.user_token";
const SETTING_ADMIN_TOKEN: &str = "server.admin_token";

fn setting(role: Role) -> &'static str {
    match role {
        Role::User => SETTING_USER_TOKEN,
        Role::Admin => SETTING_ADMIN_TOKEN,
    }
}

fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer).expect("the system's random source");
    buffer.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn new_token() -> String {
    random_hex(32)
}

pub fn ensure(library: &Library) -> vcmp_library::Result<(String, bool)> {
    let id = match library.setting(SETTING_SERVER_ID)? {
        Some(id) => id,
        None => {
            let id = random_hex(16);
            library.set_setting(SETTING_SERVER_ID, &id)?;
            id
        }
    };
    let mut made = false;
    for role in [Role::User, Role::Admin] {
        if library.setting(setting(role))?.is_none() {
            library.set_setting(setting(role), &new_token())?;
            made = true;
        }
    }
    Ok((id, made))
}

pub fn stored(library: &Library, role: Role) -> vcmp_library::Result<Option<String>> {
    library.setting(setting(role))
}

pub fn replace(library: &Library, role: Role) -> vcmp_library::Result<String> {
    let token = new_token();
    library.set_setting(setting(role), &token)?;
    Ok(token)
}

pub struct Tokens {
    configured_user: Option<String>,
    configured_admin: Option<String>,
    stored: RwLock<(String, String)>,
}

impl Tokens {
    pub fn new(
        configured_user: Option<String>,
        configured_admin: Option<String>,
        library: &Library,
    ) -> vcmp_library::Result<Self> {
        let tokens = Self {
            configured_user,
            configured_admin,
            stored: RwLock::new((String::new(), String::new())),
        };
        tokens.reload(library)?;
        Ok(tokens)
    }

    pub fn reload(&self, library: &Library) -> vcmp_library::Result<()> {
        let user = stored(library, Role::User)?.unwrap_or_default();
        let admin = stored(library, Role::Admin)?.unwrap_or_default();
        *self.stored.write().unwrap_or_else(|e| e.into_inner()) = (user, admin);
        Ok(())
    }

    pub fn is_configured(&self, role: Role) -> bool {
        match role {
            Role::User => self.configured_user.is_some(),
            Role::Admin => self.configured_admin.is_some(),
        }
    }

    pub fn role_of(&self, given: &str) -> Option<Role> {
        let stored = self.stored.read().unwrap_or_else(|e| e.into_inner());
        let admin = self.configured_admin.as_deref().unwrap_or(&stored.1);
        let user = self.configured_user.as_deref().unwrap_or(&stored.0);
        let is_admin = !admin.is_empty() && same(given.as_bytes(), admin.as_bytes());
        let is_user = !user.is_empty() && same(given.as_bytes(), user.as_bytes());
        if is_admin {
            Some(Role::Admin)
        } else if is_user {
            Some(Role::User)
        } else {
            None
        }
    }
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library() -> (tempfile::TempDir, Library) {
        let dir = tempfile::tempdir().unwrap();
        let library = Library::open(
            &dir.path().join("library.sqlite3"),
            &dir.path().join("cache"),
        )
        .unwrap();
        (dir, library)
    }

    #[test]
    fn the_id_and_tokens_are_made_once() {
        let (_dir, library) = library();
        let (id, made) = ensure(&library).unwrap();
        assert!(made);
        assert_eq!(id.len(), 32);
        let user = stored(&library, Role::User).unwrap().unwrap();
        let admin = stored(&library, Role::Admin).unwrap().unwrap();
        assert_eq!(user.len(), 64);
        assert_ne!(user, admin);
        assert_eq!(ensure(&library).unwrap(), (id, false));
        assert_eq!(stored(&library, Role::User).unwrap().unwrap(), user);
    }

    #[test]
    fn configured_tokens_replace_stored_ones_and_replaced_ones_are_read_again() {
        let (_dir, library) = library();
        ensure(&library).unwrap();
        let user = stored(&library, Role::User).unwrap().unwrap();
        let admin = stored(&library, Role::Admin).unwrap().unwrap();
        let configured = "configured-admin-token-0123".to_owned();
        let tokens = Tokens::new(None, Some(configured.clone()), &library).unwrap();
        assert_eq!(tokens.role_of(&user), Some(Role::User));
        assert_eq!(tokens.role_of(&configured), Some(Role::Admin));
        assert_eq!(tokens.role_of(&admin), None);
        assert_eq!(tokens.role_of(""), None);
        assert!(tokens.is_configured(Role::Admin));
        assert!(!tokens.is_configured(Role::User));

        let new = replace(&library, Role::User).unwrap();
        assert_eq!(tokens.role_of(&user), Some(Role::User));
        tokens.reload(&library).unwrap();
        assert_eq!(tokens.role_of(&user), None);
        assert_eq!(tokens.role_of(&new), Some(Role::User));
    }
}
